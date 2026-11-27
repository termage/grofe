//! Running a string under `/bin/sh -c`, the way tmux's `job_run` does, for
//! `run-shell`, `pipe-pane`, `copy-pipe`, `#()` sources and prompts.

use std::io;
use std::process::Stdio;

use tokio::process::{Child, Command};

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

/// Starts `cmd` holding the spawn lock, so no fork inherits a pty master
/// between its `openpty` and its `FD_CLOEXEC`, and with only its stdio
/// inherited ([`stdio_only`]). The server starts every child other than a
/// pane or a plugin (which do the same themselves) through here.
pub(crate) fn spawn(cmd: &mut Command) -> io::Result<Child> {
    stdio_only(cmd.as_std_mut());
    let _lock = pane::spawn_lock();
    cmd.spawn()
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
}
