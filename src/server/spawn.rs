//! Running a string under `/bin/sh -c`, the way tmux's `job_run` does, for
//! `run-shell`, `pipe-pane`, `copy-pipe`, `#()` sources and prompts, and
//! starting every other child of the server that is not a pane or a plugin.

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{ExitStatus, Output};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};

use crate::server::pane;

/// `/bin/sh -c script` with `argv[0]` `sh`, as tmux's `job_run`, and stderr
/// discarded. `env` replaces the environment when given and is inherited
/// when `None`. The caller sets stdio, the directory and process group, then
/// starts it with [`spawn`].
pub(crate) fn sh(script: &str, env: Option<Vec<(String, String)>>) -> Command {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg0("sh").arg("-c").arg(script).stderr(Stdio::null());
    if let Some(env) = env {
        cmd.env_clear().envs(env);
    }
    cmd
}

/// What a child's stdin, stdout or stderr is connected to.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Stdio {
    Inherit,
    Null,
    Piped,
}

impl Stdio {
    pub(crate) fn null() -> Stdio {
        Stdio::Null
    }

    pub(crate) fn piped() -> Stdio {
        Stdio::Piped
    }
}

impl From<Stdio> for std::process::Stdio {
    fn from(how: Stdio) -> Self {
        match how {
            Stdio::Inherit => std::process::Stdio::inherit(),
            Stdio::Null => std::process::Stdio::null(),
            Stdio::Piped => std::process::Stdio::piped(),
        }
    }
}

/// A [`tokio::process::Command`] that also keeps what `std` has no getter
/// for, which the macOS `posix_spawn` needs.
pub(crate) struct Command {
    inner: tokio::process::Command,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    arg0: Option<std::ffi::OsString>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    clear: bool,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    stdio: [Stdio; 3],
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    group: Option<i32>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    kill_on_drop: bool,
}

macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {$(
        pub(crate) fn $name(&mut self, $($arg: $ty),*) -> &mut Self {
            self.inner.$name($($arg),*);
            self
        }
    )*};
}

impl Command {
    pub(crate) fn new(program: impl AsRef<OsStr>) -> Command {
        Command {
            inner: tokio::process::Command::new(program),
            arg0: None,
            clear: false,
            stdio: [Stdio::Inherit; 3],
            group: None,
            kill_on_drop: false,
        }
    }

    forward! {
        arg(arg: impl AsRef<OsStr>);
        args(args: impl IntoIterator<Item = impl AsRef<OsStr>>);
        env(key: impl AsRef<OsStr>, value: impl AsRef<OsStr>);
        envs(vars: impl IntoIterator<Item = (impl AsRef<OsStr>, impl AsRef<OsStr>)>);
        env_remove(key: impl AsRef<OsStr>);
        current_dir(dir: impl AsRef<Path>);
    }

    pub(crate) fn arg0(&mut self, arg0: impl AsRef<OsStr>) -> &mut Self {
        self.arg0 = Some(arg0.as_ref().to_owned());
        self.inner.arg0(arg0);
        self
    }

    pub(crate) fn env_clear(&mut self) -> &mut Self {
        self.clear = true;
        self.inner.env_clear();
        self
    }

    pub(crate) fn stdin(&mut self, how: Stdio) -> &mut Self {
        self.stdio[0] = how;
        self.inner.stdin(how);
        self
    }

    pub(crate) fn stdout(&mut self, how: Stdio) -> &mut Self {
        self.stdio[1] = how;
        self.inner.stdout(how);
        self
    }

    pub(crate) fn stderr(&mut self, how: Stdio) -> &mut Self {
        self.stdio[2] = how;
        self.inner.stderr(how);
        self
    }

    pub(crate) fn process_group(&mut self, group: i32) -> &mut Self {
        self.group = Some(group);
        self.inner.process_group(group);
        self
    }

    pub(crate) fn kill_on_drop(&mut self, on: bool) -> &mut Self {
        self.kill_on_drop = on;
        self.inner.kill_on_drop(on);
        self
    }
}

/// A started child: its pipes, and its exit to wait for.
pub(crate) struct Child {
    pub(crate) stdin: Option<ChildStdin>,
    pub(crate) stdout: Option<ChildStdout>,
    pub(crate) stderr: Option<ChildStderr>,
    #[cfg(target_os = "macos")]
    process: mac::Process,
    #[cfg(not(target_os = "macos"))]
    process: tokio::process::Child,
}

impl Child {
    pub(crate) fn id(&self) -> Option<u32> {
        self.process.id()
    }

    pub(crate) async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.process.wait().await
    }

    /// Closes stdin, reads stdout and stderr to their end, and waits.
    pub(crate) async fn wait_with_output(mut self) -> io::Result<Output> {
        async fn read_all(pipe: Option<impl AsyncRead + Unpin>) -> io::Result<Vec<u8>> {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                pipe.read_to_end(&mut buf).await?;
            }
            Ok(buf)
        }
        drop(self.stdin.take());
        let (out, err) = (self.stdout.take(), self.stderr.take());
        let (status, stdout, stderr) = tokio::try_join!(self.wait(), read_all(out), read_all(err))?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
}

/// Starts `cmd` with only its stdio inherited, so no child holds a socket
/// or pty master another thread has not marked close-on-exec yet. On macOS
/// through `posix_spawn` and `POSIX_SPAWN_CLOEXEC_DEFAULT`; elsewhere with
/// [`stdio_only`].
pub(crate) fn spawn(cmd: &mut Command) -> io::Result<Child> {
    #[cfg(target_os = "macos")]
    return mac::spawn(cmd);
    #[cfg(not(target_os = "macos"))]
    {
        stdio_only(cmd.inner.as_std_mut());
        let mut child = cmd.inner.spawn()?;
        Ok(Child {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            process: child,
        })
    }
}

/// Marks every fd above 2 close-on-exec in the child, as a pane's spawn
/// does ([`pane::cloexec_inherited_fds`]): macOS sets `FD_CLOEXEC` on a new
/// socket with a separate call, so a child started meanwhile on another
/// thread would hold that socket (a client's connection, a remote link)
/// open for its whole life.
pub(crate) fn stdio_only(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    let limit = pane::nofile_limit();
    // SAFETY: the closure only makes async-signal-safe `fcntl` calls.
    unsafe {
        cmd.pre_exec(move || {
            pane::cloexec_inherited_fds(limit);
            Ok(())
        });
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::collections::BTreeMap;
    use std::ffi::{CString, OsString};
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::ExitStatusExt;
    use std::ptr::null_mut;
    use std::sync::{Arc, Mutex};

    use nix::libc::{
        self, c_char, c_int, posix_spawn_file_actions_t as Actions, posix_spawnattr_t,
    };
    use tokio::sync::watch;

    use super::*;

    unsafe extern "C" {
        fn posix_spawn_file_actions_addchdir_np(fa: *mut Actions, path: *const c_char) -> c_int;
        fn posix_spawn_file_actions_addinherit_np(fa: *mut Actions, fd: c_int) -> c_int;
    }

    fn check(ret: c_int) -> io::Result<()> {
        match ret {
            0 => Ok(()),
            e => Err(io::Error::from_raw_os_error(e)),
        }
    }

    fn cstr(s: &OsStr) -> io::Result<CString> {
        CString::new(s.as_bytes()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
    }

    struct Attr(posix_spawnattr_t);

    impl Drop for Attr {
        fn drop(&mut self) {
            // SAFETY: initialised in `spawn`, destroyed once.
            unsafe { libc::posix_spawnattr_destroy(&mut self.0) };
        }
    }

    struct FileActions(Actions);

    impl Drop for FileActions {
        fn drop(&mut self) {
            // SAFETY: initialised in `spawn`, destroyed once.
            unsafe { libc::posix_spawn_file_actions_destroy(&mut self.0) };
        }
    }

    /// `program` found as `execvp` finds it: on the child's `PATH`.
    fn resolve(program: &OsStr, env: &BTreeMap<OsString, OsString>) -> io::Result<CString> {
        if program.as_bytes().contains(&b'/') {
            return cstr(program);
        }
        let path = env.get(OsStr::new("PATH"));
        let path = path.map_or(&b"/usr/bin:/bin"[..], |p| p.as_bytes());
        for dir in path.split(|&b| b == b':') {
            let file = Path::new(OsStr::from_bytes(if dir.is_empty() { b"." } else { dir }));
            let file = file.join(program);
            let meta = std::fs::metadata(&file);
            if meta.is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0) {
                return cstr(file.as_os_str());
            }
        }
        Err(io::Error::from_raw_os_error(libc::ENOENT))
    }

    pub(super) fn spawn(cmd: &Command) -> io::Result<Child> {
        let std = cmd.inner.as_std();
        let mut env: BTreeMap<OsString, OsString> = if cmd.clear {
            BTreeMap::new()
        } else {
            std::env::vars_os().collect()
        };
        for (key, value) in std.get_envs() {
            match value {
                Some(value) => env.insert(key.to_owned(), value.to_owned()),
                None => env.remove(key),
            };
        }
        let program = resolve(std.get_program(), &env)?;
        let arg0 = cmd.arg0.as_deref().unwrap_or(std.get_program());
        let argv = std::iter::once(arg0).chain(std.get_args()).map(cstr);
        let argv = argv.collect::<io::Result<Vec<_>>>()?;
        let envp = env
            .iter()
            .map(|(k, v)| cstr(&[k.as_os_str(), v.as_os_str()].join(OsStr::new("="))));
        let envp = envp.collect::<io::Result<Vec<_>>>()?;

        let mut attr = Attr(null_mut());
        // SAFETY: `attr.0` is valid for writes; `Attr` destroys it.
        check(unsafe { libc::posix_spawnattr_init(&mut attr.0) })?;
        let mut actions = FileActions(null_mut());
        // SAFETY: as above, for `FileActions`.
        check(unsafe { libc::posix_spawn_file_actions_init(&mut actions.0) })?;
        let fa = &mut actions.0;
        // As std: no signal blocked, and SIGPIPE (which std ignores) default.
        let mut flags = libc::POSIX_SPAWN_CLOEXEC_DEFAULT
            | libc::POSIX_SPAWN_SETSIGMASK
            | libc::POSIX_SPAWN_SETSIGDEF;
        // SAFETY: plain calls on initialised attributes and signal sets.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            check(libc::posix_spawnattr_setsigmask(&mut attr.0, &set))?;
            libc::sigaddset(&mut set, libc::SIGPIPE);
            check(libc::posix_spawnattr_setsigdefault(&mut attr.0, &set))?;
            if let Some(group) = cmd.group {
                flags |= libc::POSIX_SPAWN_SETPGROUP;
                check(libc::posix_spawnattr_setpgroup(&mut attr.0, group))?;
            }
            check(libc::posix_spawnattr_setflags(
                &mut attr.0,
                flags as libc::c_short,
            ))?;
        }
        if let Some(dir) = std.get_current_dir() {
            let dir = cstr(dir.as_os_str())?;
            // SAFETY: the actions copy `dir`.
            check(unsafe { posix_spawn_file_actions_addchdir_np(fa, dir.as_ptr()) })?;
        }
        // Under CLOEXEC_DEFAULT only fds named by a file action survive.
        let mut ours: [Option<OwnedFd>; 3] = Default::default();
        let mut theirs = Vec::new();
        for (fd, how) in cmd.stdio.iter().enumerate() {
            let target = fd as c_int;
            // SAFETY: `fa` is initialised; the actions copy the path.
            match how {
                Stdio::Inherit => {
                    check(unsafe { posix_spawn_file_actions_addinherit_np(fa, target) })?
                }
                Stdio::Null => {
                    let mode = if fd == 0 {
                        libc::O_RDONLY
                    } else {
                        libc::O_WRONLY
                    };
                    let null = c"/dev/null".as_ptr();
                    check(unsafe {
                        libc::posix_spawn_file_actions_addopen(fa, target, null, mode, 0)
                    })?
                }
                Stdio::Piped => {
                    let (read, write) = io::pipe()?;
                    let (mine, child): (OwnedFd, OwnedFd) = match fd {
                        0 => (write.into(), read.into()),
                        _ => (read.into(), write.into()),
                    };
                    let raw = child.as_raw_fd();
                    check(unsafe { libc::posix_spawn_file_actions_adddup2(fa, raw, target) })?;
                    ours[fd] = Some(mine);
                    theirs.push(child);
                }
            }
        }

        let ptrs = |v: &[CString]| -> Vec<*mut c_char> {
            let p = v.iter().map(|s| s.as_ptr().cast_mut());
            p.chain(std::iter::once(null_mut())).collect()
        };
        let (argv_p, envp_p) = (ptrs(&argv), ptrs(&envp));
        let mut pid = 0;
        // SAFETY: every pointer is valid and NUL-terminated for the call.
        check(unsafe {
            libc::posix_spawn(
                &mut pid,
                program.as_ptr(),
                fa,
                &attr.0,
                argv_p.as_ptr(),
                envp_p.as_ptr(),
            )
        })?;
        drop(theirs);
        let process = Process::start(pid, cmd.kill_on_drop)?;
        let [stdin, stdout, stderr] = ours;
        Ok(Child {
            stdin: stdin
                .map(|fd| ChildStdin::from_std(fd.into()))
                .transpose()?,
            stdout: stdout
                .map(|fd| ChildStdout::from_std(fd.into()))
                .transpose()?,
            stderr: stderr
                .map(|fd| ChildStderr::from_std(fd.into()))
                .transpose()?,
            process,
        })
    }

    /// The exit of a `posix_spawn`ed child, reaped on its own thread.
    pub(super) struct Process {
        pid: libc::pid_t,
        kill_on_drop: bool,
        /// Set under the lock as the child is reaped, so a kill made while
        /// it is unset still reaches this child (or its zombie), never a
        /// reused pid.
        reaped: Arc<Mutex<bool>>,
        status: watch::Receiver<Option<Result<ExitStatus, i32>>>,
    }

    fn lock(m: &Mutex<bool>) -> std::sync::MutexGuard<'_, bool> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    impl Process {
        fn start(pid: libc::pid_t, kill_on_drop: bool) -> io::Result<Process> {
            let (tx, status) = watch::channel(None);
            let reaped = Arc::new(Mutex::new(false));
            let flag = Arc::clone(&reaped);
            let reaper = move || {
                // Waits without reaping: the pid stays this child's until
                // `waitpid` below, under the lock.
                // SAFETY: a zeroed siginfo_t is valid, and waitid only writes it.
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let (exited, nowait) = (libc::WEXITED, libc::WNOWAIT);
                // SAFETY: `info` outlives the call.
                while unsafe {
                    libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, exited | nowait)
                } != 0
                    && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
                {
                }
                let mut done = lock(&flag);
                let mut raw = 0;
                // SAFETY: `raw` outlives the call.
                let got = match unsafe { libc::waitpid(pid, &mut raw, 0) } {
                    -1 => Err(io::Error::last_os_error()
                        .raw_os_error()
                        .unwrap_or(libc::ECHILD)),
                    _ => Ok(ExitStatus::from_raw(raw)),
                };
                *done = true;
                drop(done);
                let _ = tx.send(Some(got));
            };
            let thread = std::thread::Builder::new().name("grofe-reap".into());
            if let Err(e) = thread.stack_size(64 * 1024).spawn(reaper) {
                // SAFETY: plain syscalls on our own unreaped child.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                    libc::waitpid(pid, null_mut(), 0);
                }
                return Err(e);
            }
            Ok(Process {
                pid,
                kill_on_drop,
                reaped,
                status,
            })
        }

        pub(super) fn id(&self) -> Option<u32> {
            u32::try_from(self.pid).ok()
        }

        pub(super) async fn wait(&mut self) -> io::Result<ExitStatus> {
            let got = self.status.wait_for(Option::is_some).await;
            let got = got.map_err(|_| io::Error::other("child reaper ended"))?;
            (*got)
                .unwrap_or(Err(libc::ECHILD))
                .map_err(io::Error::from_raw_os_error)
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            let reaped = lock(&self.reaped);
            if self.kill_on_drop && !*reaped {
                // SAFETY: not reaped, so `pid` is still this child.
                unsafe { libc::kill(self.pid, libc::SIGKILL) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn out(mut cmd: Command) -> String {
        cmd.stdin(Stdio::null()).stdout(Stdio::piped());
        let child = spawn(&mut cmd).expect("spawn");
        let out = child.wait_with_output().await.expect("wait");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[tokio::test]
    async fn the_shell_is_called_sh_and_stderr_goes_nowhere() {
        assert_eq!(out(sh("echo $0; echo oops >&2", None)).await, "sh\n");
    }

    #[tokio::test]
    async fn given_env_replaces_the_environment_and_none_inherits_it() {
        let only = vec![("ONLY".to_string(), "x".to_string())];
        let got = out(sh("echo \"$ONLY${HOME:-none}\"", Some(only))).await;
        assert_eq!(got, "xnone\n");
        let inherited = out(sh("echo ${HOME:+set}", None)).await;
        assert_eq!(inherited, "set\n");
    }

    /// A socket caught between its creation and its `FD_CLOEXEC` (macOS sets
    /// it with a separate call) stays out of the child.
    #[tokio::test]
    async fn a_socket_without_close_on_exec_stays_out_of_the_child() {
        use std::os::fd::AsRawFd;
        let (sock, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let fd = sock.as_raw_fd();
        use nix::fcntl::{FcntlArg, FdFlag, fcntl};
        fcntl(&sock, FcntlArg::F_SETFD(FdFlag::empty())).unwrap();
        let script = format!("[ -e /dev/fd/{fd} ] && echo held || echo closed");
        assert_eq!(out(sh(&script, None)).await, "closed\n");
    }

    /// The child reads its stdin pipe and writes both output pipes, in the
    /// directory, environment and process group given, and a program
    /// without a slash is found on `PATH`.
    #[tokio::test]
    async fn a_child_gets_its_pipes_dir_env_and_group() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "read l; echo \"$l $V $(pwd -P)\"; echo e >&2; ps -o pgid= -p $$",
        ])
        .env("V", "v")
        .current_dir(dir.path())
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = spawn(&mut cmd).expect("spawn");
        let pid = child.id().expect("pid");
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(b"in\n").await.expect("write");
        drop(stdin);
        let out = child.wait_with_output().await.expect("wait");
        assert!(out.status.success());
        let real = dir.path().canonicalize().expect("canonical");
        let text = String::from_utf8_lossy(&out.stdout);
        let want = format!("in v {}\n", real.display());
        assert_eq!(text.lines().next(), want.lines().next());
        assert_eq!(
            text.lines().nth(1).map(str::trim),
            Some(pid.to_string().as_str())
        );
        assert_eq!(out.stderr, b"e\n");
    }

    /// A program that is not there fails the spawn, and `kill_on_drop` ends
    /// a child still running when it drops.
    #[tokio::test]
    async fn a_missing_program_fails_and_kill_on_drop_kills() {
        let err = spawn(&mut Command::new("grofe-no-such-program")).err();
        assert_eq!(err.map(|e| e.kind()), Some(io::ErrorKind::NotFound));
        let mut cmd = Command::new("sleep");
        cmd.arg("30").kill_on_drop(true);
        let child = spawn(&mut cmd).expect("spawn");
        let pid = nix::unistd::Pid::from_raw(child.id().expect("pid") as i32);
        drop(child);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while nix::sys::signal::kill(pid, None).is_ok() {
            assert!(std::time::Instant::now() < deadline, "still running");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}
