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
        }
    }

    forward! {
        arg(arg: impl AsRef<OsStr>);
        args(args: impl IntoIterator<Item = impl AsRef<OsStr>>);
        env(key: impl AsRef<OsStr>, value: impl AsRef<OsStr>);
        envs(vars: impl IntoIterator<Item = (impl AsRef<OsStr>, impl AsRef<OsStr>)>);
        env_remove(key: impl AsRef<OsStr>);
        current_dir(dir: impl AsRef<Path>);
        kill_on_drop(on: bool);
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
mod mac;

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
        cmd.args(["-c", "read l; echo \"$l $V $(pwd -P)\"; echo e >&2"])
            .env("V", "v")
            .current_dir(dir.path())
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = spawn(&mut cmd).expect("spawn");
        let pid = nix::unistd::Pid::from_raw(child.id().expect("pid") as i32);
        // Still running: it waits on its stdin.
        assert_eq!(nix::unistd::getpgid(Some(pid)), Ok(pid));
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(b"in\n").await.expect("write");
        drop(stdin);
        let out = child.wait_with_output().await.expect("wait");
        assert!(out.status.success());
        let real = dir.path().canonicalize().expect("canonical");
        let text = String::from_utf8_lossy(&out.stdout);
        assert_eq!(text, format!("in v {}\n", real.display()));
        assert_eq!(out.stderr, b"e\n");
    }

    /// The child starts as std starts one: SIGPIPE (which Rust ignores) at
    /// its default, and no signal blocked even when the spawning thread
    /// blocks one.
    #[tokio::test]
    async fn a_child_has_sigpipe_at_default_and_no_signal_blocked() {
        use nix::sys::signal::{SigSet, SigmaskHow, Signal, pthread_sigmask};
        use std::os::unix::process::ExitStatusExt;
        let run = |script: &str| {
            let mut cmd = sh(script, None);
            cmd.stdout(Stdio::piped());
            spawn(&mut cmd).expect("spawn")
        };
        let out = run("kill -PIPE $$; echo alive").wait_with_output().await;
        let out = out.expect("wait");
        assert_eq!(out.status.signal(), Some(Signal::SIGPIPE as i32));
        assert!(out.stdout.is_empty());
        let mut usr1 = SigSet::empty();
        usr1.add(Signal::SIGUSR1);
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&usr1), None).expect("block");
        let child = run("kill -USR1 $$; echo alive");
        pthread_sigmask(SigmaskHow::SIG_UNBLOCK, Some(&usr1), None).expect("unblock");
        let out = child.wait_with_output().await.expect("wait");
        assert_eq!(out.status.signal(), Some(Signal::SIGUSR1 as i32));
        assert!(out.stdout.is_empty());
    }

    /// Whether the child `pid` has exited (`Some(true)`, a zombie until it
    /// is reaped) or runs (`Some(false)`); `None` once it is reaped. Never
    /// reaps it.
    fn exited(pid: u32) -> Option<bool> {
        use nix::libc;
        // SAFETY: a zeroed siginfo_t is valid, and waitid only writes it.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let flags = libc::WEXITED | libc::WNOWAIT | libc::WNOHANG;
        // SAFETY: `info` outlives the call.
        let r = unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, flags) };
        (r == 0).then_some(info.si_signo == libc::SIGCHLD)
    }

    async fn until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "never {what}");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    /// A child that exited stays a zombie until it is waited on, so its pid
    /// and group id stay its own; once waited on it has no id.
    #[tokio::test]
    async fn an_exited_child_is_reaped_only_by_its_wait() {
        let mut child = spawn(&mut sh("exit 3", None)).expect("spawn");
        let pid = child.id().expect("pid");
        until("exited", || exited(pid) == Some(true)).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(exited(pid), Some(true), "reaped before its wait");
        assert_eq!(child.wait().await.expect("wait").code(), Some(3));
        assert_eq!(child.id(), None);
        assert_eq!(exited(pid), None);
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
        let pid = child.id().expect("pid");
        drop(child);
        until("killed", || exited(pid) != Some(false)).await;
    }
}
