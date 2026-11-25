//! `--config` and `$GROFE_CONFIG` on the binary's side: resolving the file
//! once, and telling the user when the running server loaded another one.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use grofe::config;
use grofe::protocol::{ClientCtx, Dialect, Hello, Reply, encode_hello};

use super::request_with;

/// The `--config` file as an absolute path; unset without the flag.
static FLAG: OnceLock<PathBuf> = OnceLock::new();

/// The `--config-dir` folder as an absolute path; unset without the flag.
static DIR_FLAG: OnceLock<PathBuf> = OnceLock::new();

/// Resolves `--config` (else `$GROFE_CONFIG`) to an absolute path and records
/// it for this process's config reads. It is not exported as a variable: the
/// environment this CLI forwards would carry it into every pane. A server
/// this CLI starts gets it as an argument ([`server_args`]). `dir` is
/// `--config-dir` (else `$GROFE_CONFIG_DIR`), handled the same way.
pub fn apply(flag: Option<&str>, dir: Option<&str>) -> Result<(), String> {
    apply_dir(dir)?;
    let path = match flag {
        Some(arg) => {
            let path = config::resolve_from_process(arg)?;
            if path.is_dir() {
                return Err(format!("--config: {} is a directory", path.display()));
            }
            path
        }
        None => match config::env_config_checked()? {
            Some(path) => path,
            None => return Ok(()),
        },
    };
    if flag.is_some() {
        let _ = FLAG.set(path.clone());
    }
    config::set_process_config(path);
    Ok(())
}

/// Resolves and checks the folder of `--config-dir`, else
/// `$GROFE_CONFIG_DIR`: a path that is not a folder is an error naming it.
fn apply_dir(flag: Option<&str>) -> Result<(), String> {
    let path = match flag {
        Some(arg) => {
            let path = config::resolve_from_process(arg)?;
            config::Env::from_dir(&path, None).map_err(|e| format!("--config-dir: {e}"))?;
            let _ = DIR_FLAG.set(path.clone());
            path
        }
        None => match config::env_config_dir_checked()? {
            Some(path) => path,
            None => return Ok(()),
        },
    };
    config::set_process_config_dir(path);
    Ok(())
}

/// Under the tmux name, removes `--config-dir <dir>` from the options
/// before the command and returns its value. tmux's own options that take
/// a value are skipped with it, so `-f --config-dir` stays a file name.
// ponytail: a value glued to a combined flag cluster (`-2f--config-dir`) is not seen.
pub fn take_config_dir(args: &[String]) -> (Option<String>, Vec<String>) {
    let mut out = Vec::with_capacity(args.len());
    let mut dir = None;
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        match arg.as_str() {
            "--config-dir" if args.get(i + 1).is_some() => {
                dir = args.get(i + 1).cloned();
                i += 2;
                continue;
            }
            "-c" | "-f" | "-L" | "-S" | "-T" => {
                out.extend(args[i..].iter().take(2).cloned());
                i += 2;
                continue;
            }
            "--" => {}
            a if a.starts_with('-') && a.len() > 1 => {
                out.push(arg.clone());
                i += 1;
                continue;
            }
            _ => {}
        }
        break;
    }
    out.extend(args[i.min(args.len())..].iter().cloned());
    (dir, out)
}

/// The arguments that give a server this CLI starts its config file and
/// folder.
pub fn server_args() -> Vec<std::ffi::OsString> {
    let mut args = Vec::new();
    if let Some(file) = config::process_config() {
        args.extend(["--config".into(), file.into()]);
    }
    if let Some(dir) = config::process_config_dir() {
        args.extend(["--config-dir".into(), dir.into()]);
    }
    args
}

/// Warns about a server on `socket` running other config than `--config`,
/// except for the verbs that never need it.
pub fn warn_for(verb: &str, socket: &Path) {
    if !matches!(verb, "__server" | "show-config" | "import-tmux" | "pack") {
        warn_if_other(socket);
    }
}

/// How long the server gets to answer [`warn_if_other`].
const TIMEOUT: Duration = Duration::from_secs(5);

/// Tells the user when the server running on `socket` loaded other config
/// files than the `--config` this CLI was given, or uses another folder
/// than its `--config-dir`. Never starts a server and never fails the
/// command.
pub fn warn_if_other(socket: &Path) {
    let (mine, my_dir) = (FLAG.get(), DIR_FLAG.get());
    if mine.is_none() && my_dir.is_none() {
        return;
    }
    let Ok(stream) = UnixStream::connect(socket) else {
        return;
    };
    let Some(theirs) = ask_files(stream) else {
        return;
    };
    if let Some(mine) = mine
        && !theirs.files.iter().any(|f| same_file(f, mine))
    {
        let named: Vec<_> = theirs
            .files
            .iter()
            .map(|f| f.display().to_string())
            .collect();
        let named = if named.is_empty() {
            "no config file".to_string()
        } else {
            named.join(", ")
        };
        eprintln!("grofe: server uses {named}");
    }
    if let Some(my_dir) = my_dir
        && !theirs.dir.as_deref().is_some_and(|d| same_file(d, my_dir))
    {
        let named = match &theirs.dir {
            Some(d) => format!("config folder {}", d.display()),
            None => "no config folder".to_string(),
        };
        eprintln!("grofe: server uses {named}");
    }
}

/// The config folder of the server on `socket` (for a server too old to
/// name it, the folder of its config file), or `None` when no server
/// answers or it has none.
pub fn server_config_dir(socket: &Path) -> Option<PathBuf> {
    let stream = UnixStream::connect(socket).ok()?;
    let theirs = ask_files(stream)?;
    theirs
        .dir
        .or_else(|| theirs.files.first()?.parent().map(Path::to_path_buf))
}

/// What a `__config-file` reply says: the files the server loaded and its
/// config folder.
struct ServerConfig {
    files: Vec<PathBuf>,
    dir: Option<PathBuf>,
}

/// Splits a `__config-file` reply into its files and its folder line.
fn parse_reply(out: &str) -> ServerConfig {
    let mut theirs = ServerConfig {
        files: Vec::new(),
        dir: None,
    };
    for line in out.lines() {
        match line.strip_prefix(config::CONFIG_DIR_LINE) {
            Some(dir) => theirs.dir = Some(PathBuf::from(dir)),
            None => theirs.files.push(PathBuf::from(line)),
        }
    }
    theirs
}

/// The files the server loaded and its folder, asked in grofe's dialect
/// whatever name this CLI runs as.
fn ask_files(stream: UnixStream) -> Option<ServerConfig> {
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(TIMEOUT)).ok()?;
    let ctx = ClientCtx {
        cwd: PathBuf::from("/"),
        pane: None,
        env: Vec::new(),
        client: None,
        dialect: Dialect::Grofe,
        request: Default::default(),
    };
    let hello = encode_hello(&Hello::Control(ctx)).ok()?;
    let _done = super::HangUp(stream.try_clone().ok());
    match request_with(stream, &hello, &["__config-file".to_string()]) {
        Ok(Reply::Ok { out, .. }) => Some(parse_reply(&out)),
        _ => None,
    }
}

/// Whether two paths name one file: compared resolved when both exist
/// (`/tmp` is `/private/tmp` on macOS), else as given.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => normalize(a) == normalize(b),
    }
}

/// `path` with `.` and `..` resolved lexically, for files that do not exist.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reply_splits_into_files_and_the_folder() {
        let theirs = parse_reply("/a/x.toml\n/b/y.conf\ndir:/c d/ø\n");
        assert_eq!(
            theirs.files,
            [PathBuf::from("/a/x.toml"), PathBuf::from("/b/y.conf")]
        );
        assert_eq!(theirs.dir, Some(PathBuf::from("/c d/ø")));
        assert_eq!(parse_reply("/a/x.toml\n").dir, None);
    }

    #[test]
    fn config_dir_is_taken_from_the_tmux_options_only() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (dir, rest) = take_config_dir(&v(&["-L", "x", "--config-dir", "d", "new", "-d"]));
        assert_eq!(dir.as_deref(), Some("d"));
        assert_eq!(rest, v(&["-L", "x", "new", "-d"]));
        let args = v(&["-f", "--config-dir", "new", "--config-dir", "d"]);
        assert_eq!(take_config_dir(&args), (None, args.clone()));
        let args = v(&["--config-dir"]);
        assert_eq!(take_config_dir(&args), (None, args.clone()));
    }

    #[test]
    fn missing_files_compare_after_lexical_normalising() {
        assert!(same_file(
            Path::new("/nonexistent-grofe/a/../a/x.toml"),
            Path::new("/nonexistent-grofe/a/x.toml")
        ));
        assert!(same_file(
            Path::new("nonexistent-grofe/a/../a/x.toml"),
            Path::new("nonexistent-grofe/a/x.toml")
        ));
        assert!(!same_file(
            Path::new("/nonexistent-grofe/a/x.toml"),
            Path::new("/nonexistent-grofe/b/x.toml")
        ));
    }
}
