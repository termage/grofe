use crate::lua::json::merge;
use crate::pack::lock::Lock;
use crate::pack::manifest::{Kind, default_options, detect};
use crate::pack::spec::{Spec, pack_root};
use crate::server::{State, clients};
use std::path::{Path, PathBuf};

/// Loads every enabled, locked and installed pack in name order. A failing
/// pack goes to the log and never stops the others.
pub(crate) fn load_all(state: &mut State, specs: &[Spec], config_dir: &Path) {
    let lock = match Lock::read(&config_dir.join("pack-lock.toml")) {
        Ok(l) => l,
        Err(e) => return report(state, format!("pack: {e}")),
    };
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let xdg = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let root = pack_root(&home, xdg.as_deref());
    for (spec, dir) in selected(specs, &lock, &root) {
        let loaded = match detect(&dir) {
            Ok(Kind::Luau) => load_luau(state, spec, &dir),
            Ok(Kind::Tmux) => {
                crate::server::hooks::run_at_load(state, tmux_cmds(&dir));
                Ok(())
            }
            Err(e) => Err(e),
        };
        if let Err(e) = loaded {
            report(state, format!("pack {}: {e}", spec.name));
        }
    }
}

/// The manifest's option defaults under the user's `options`.
fn load_luau(state: &mut State, spec: &Spec, dir: &Path) -> Result<(), String> {
    let mut options = default_options(dir)?;
    merge(
        &mut options,
        serde_json::Value::Object(spec.options.clone()),
    );
    crate::server::lua_host::load_pack(state, &spec.name, dir, &options)
}

/// A pack that failed goes to the log and to `show-messages`.
fn report(state: &mut State, line: String) {
    eprintln!("grofe {line}");
    clients::log_error(state, "pack", &line);
}

/// The enabled specs that are locked and cloned under `root`, by name.
fn selected<'a>(specs: &'a [Spec], lock: &Lock, root: &Path) -> Vec<(&'a Spec, PathBuf)> {
    let mut out: Vec<(&Spec, PathBuf)> = specs
        .iter()
        .filter(|s| s.enabled && lock.plugins.contains_key(&s.name))
        .map(|s| (s, root.join(&s.name)))
        .filter(|(_, dir)| dir.is_dir())
        .collect();
    out.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    out
}

/// One `run-shell` per `*.tmux` file in `dir`, sorted by file name.
fn tmux_cmds(dir: &Path) -> Vec<Vec<String>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "tmux"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| vec!["run-shell".to_string(), p.to_string_lossy().into_owned()])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::lock::Locked;

    fn spec(name: &str, enabled: bool) -> Spec {
        Spec {
            name: name.into(),
            src: format!("https://x/{name}"),
            rev: None,
            enabled,
            options: Default::default(),
        }
    }

    #[test]
    fn test_only_locked_installed_enabled_load() {
        let root = tempfile::tempdir().unwrap();
        for n in ["b", "a", "off", "unlocked"] {
            std::fs::create_dir(root.path().join(n)).unwrap();
        }
        let mut lock = Lock::default();
        for n in ["a", "b", "off", "missing"] {
            lock.plugins.insert(n.into(), Locked::default());
        }
        let specs = [
            spec("b", true),
            spec("a", true),
            spec("off", false),
            spec("unlocked", true),
            spec("missing", true),
        ];
        let got: Vec<String> = selected(&specs, &lock, root.path())
            .into_iter()
            .map(|(s, _)| s.name.clone())
            .collect();
        assert_eq!(got, ["a", "b"]);
    }

    #[test]
    fn test_tmux_pack_queues_run_shell_sorted() {
        let d = tempfile::tempdir().unwrap();
        for f in ["z.tmux", "a.tmux", "readme.md"] {
            std::fs::write(d.path().join(f), "").unwrap();
        }
        let cmds = tmux_cmds(d.path());
        let want: Vec<Vec<String>> = ["a.tmux", "z.tmux"]
            .iter()
            .map(|f| {
                vec![
                    "run-shell".into(),
                    d.path().join(f).to_string_lossy().into_owned(),
                ]
            })
            .collect();
        assert_eq!(cmds, want);
    }
}
