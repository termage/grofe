//! Helper children on macOS: started with `posix_spawn` and
//! `POSIX_SPAWN_CLOEXEC_DEFAULT`, so only the stdio its file actions name
//! survives into the child, with no fork and no fd loop.

use std::collections::BTreeMap;
use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::ptr::null_mut;
use std::sync::{Arc, Mutex, MutexGuard};

use nix::libc::{self, c_char, c_int, pid_t, posix_spawn_file_actions_t, posix_spawnattr_t};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::watch;

use super::{Child, Command, Stdio};

unsafe extern "C" {
    fn posix_spawn_file_actions_addchdir_np(
        fa: *mut posix_spawn_file_actions_t,
        path: *const c_char,
    ) -> c_int;
    fn posix_spawn_file_actions_addinherit_np(
        fa: *mut posix_spawn_file_actions_t,
        fd: c_int,
    ) -> c_int;
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

pub(super) fn spawn(cmd: &Command) -> io::Result<Child> {
    let env = env_of(cmd);
    let program = resolve(cmd.inner.as_std().get_program(), &env)?;
    let (argv, envp) = strings(cmd, &env)?;
    let attr = attrs(cmd)?;
    let (actions, pipes) = file_actions(cmd)?;
    let ptrs = |v: &[CString]| -> Vec<*mut c_char> {
        let p = v.iter().map(|s| s.as_ptr().cast_mut());
        p.chain(std::iter::once(null_mut())).collect()
    };
    let (argv, envp) = (ptrs(&argv), ptrs(&envp));
    let mut pid = 0;
    // SAFETY: every pointer is valid and NUL-terminated for the call.
    check(unsafe {
        libc::posix_spawn(
            &mut pid,
            program.as_ptr(),
            &actions.0,
            &attr.0,
            argv.as_ptr(),
            envp.as_ptr(),
        )
    })?;
    wrap(pid, cmd, pipes)
}

/// The child's environment as std builds it: the server's unless cleared,
/// with the command's changes.
fn env_of(cmd: &Command) -> BTreeMap<OsString, OsString> {
    let mut env: BTreeMap<OsString, OsString> = if cmd.clear {
        BTreeMap::new()
    } else {
        std::env::vars_os().collect()
    };
    for (key, value) in cmd.inner.as_std().get_envs() {
        match value {
            Some(value) => env.insert(key.to_owned(), value.to_owned()),
            None => env.remove(key),
        };
    }
    env
}

/// `argv`, its first word `arg0` when set, and `envp`.
fn strings(
    cmd: &Command,
    env: &BTreeMap<OsString, OsString>,
) -> io::Result<(Vec<CString>, Vec<CString>)> {
    let std = cmd.inner.as_std();
    let arg0 = cmd.arg0.as_deref().unwrap_or(std.get_program());
    let argv = std::iter::once(arg0).chain(std.get_args()).map(cstr);
    let envp = env.iter().map(|(k, v)| {
        let mut pair = k.clone();
        pair.push("=");
        pair.push(v);
        cstr(&pair)
    });
    Ok((
        argv.collect::<io::Result<_>>()?,
        envp.collect::<io::Result<_>>()?,
    ))
}

/// `program` found as `execvp` finds it: on the child's `PATH`.
// ponytail: not all of execvp. A file found but not executable is ENOENT,
// not EACCES; a script without `#!` is not retried under `/bin/sh`
// (ENOEXEC); and a relative `PATH` entry is taken from the server's
// directory, not the command's `current_dir`.
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

struct Attr(posix_spawnattr_t);

impl Drop for Attr {
    fn drop(&mut self) {
        // SAFETY: initialised by `attrs`, destroyed once.
        unsafe { libc::posix_spawnattr_destroy(&mut self.0) };
    }
}

/// Only the stdio named by a file action survives; as std, no signal is
/// blocked and SIGPIPE (which std ignores) is back at its default.
fn attrs(cmd: &Command) -> io::Result<Attr> {
    let mut attr = Attr(null_mut());
    // SAFETY: `attr.0` is valid for writes; `Attr` destroys it.
    check(unsafe { libc::posix_spawnattr_init(&mut attr.0) })?;
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
    Ok(attr)
}

struct FileActions(posix_spawn_file_actions_t);

impl Drop for FileActions {
    fn drop(&mut self) {
        // SAFETY: initialised by `file_actions`, destroyed once.
        unsafe { libc::posix_spawn_file_actions_destroy(&mut self.0) };
    }
}

/// The server's ends of the child's pipes, ready for tokio before the
/// child starts, and the child's ends, open until it has.
struct Pipes {
    theirs: Vec<OwnedFd>,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
}

/// The directory change and the stdio of `cmd` as file actions.
fn file_actions(cmd: &Command) -> io::Result<(FileActions, Pipes)> {
    let mut actions = FileActions(null_mut());
    // SAFETY: `actions.0` is valid for writes; `FileActions` destroys it.
    check(unsafe { libc::posix_spawn_file_actions_init(&mut actions.0) })?;
    let fa = &mut actions.0;
    if let Some(dir) = cmd.inner.as_std().get_current_dir() {
        let dir = cstr(dir.as_os_str())?;
        // SAFETY: the actions copy `dir`.
        check(unsafe { posix_spawn_file_actions_addchdir_np(fa, dir.as_ptr()) })?;
    }
    let mut ours: [Option<OwnedFd>; 3] = Default::default();
    let mut theirs = Vec::new();
    for (fd, how) in cmd.stdio.iter().enumerate() {
        let target = fd as c_int;
        // SAFETY (each call): `fa` is initialised; the actions copy the path.
        match how {
            Stdio::Inherit => check(unsafe { posix_spawn_file_actions_addinherit_np(fa, target) })?,
            Stdio::Null => {
                let mode = if fd == 0 {
                    libc::O_RDONLY
                } else {
                    libc::O_WRONLY
                };
                let null = c"/dev/null".as_ptr();
                check(unsafe { libc::posix_spawn_file_actions_addopen(fa, target, null, mode, 0) })?
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
    let [stdin, stdout, stderr] = ours;
    let pipes = Pipes {
        theirs,
        stdin: stdin
            .map(|fd| ChildStdin::from_std(fd.into()))
            .transpose()?,
        stdout: stdout
            .map(|fd| ChildStdout::from_std(fd.into()))
            .transpose()?,
        stderr: stderr
            .map(|fd| ChildStderr::from_std(fd.into()))
            .transpose()?,
    };
    Ok((actions, pipes))
}

/// The started child `pid` as a [`Child`], closing its ends of the pipes.
fn wrap(pid: pid_t, cmd: &Command, pipes: Pipes) -> io::Result<Child> {
    let Pipes {
        theirs,
        stdin,
        stdout,
        stderr,
    } = pipes;
    drop(theirs);
    Ok(Child {
        stdin,
        stdout,
        stderr,
        process: Process::start(pid, cmd.inner.get_kill_on_drop())?,
    })
}

/// What the reaper thread and its [`Process`] share.
#[derive(Default)]
struct Shared {
    /// The child has exited. Until it is reaped it is a zombie, so its pid
    /// and group id are still its own.
    exited: bool,
    /// Reaped, with what `waitpid` gave.
    status: Option<Result<ExitStatus, i32>>,
    /// The [`Process`] is gone, so the reaper reaps the child on its exit.
    dropped: bool,
}

/// A `posix_spawn`ed child, reaped as tokio reaps one: when it is waited
/// on, or once it has exited after it was dropped.
pub(super) struct Process {
    pid: pid_t,
    kill_on_drop: bool,
    shared: Arc<Mutex<Shared>>,
    exited: watch::Receiver<bool>,
}

fn lock(m: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Blocks until `pid` exits, leaving it unreaped.
fn wait_exited(pid: pid_t) {
    // SAFETY: a zeroed siginfo_t is valid, and waitid only writes it.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let flags = libc::WEXITED | libc::WNOWAIT;
    // SAFETY: `info` outlives the call.
    while unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, flags) } != 0
        && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
    {}
}

/// Reaps `pid`, which has exited.
fn reap(pid: pid_t) -> Result<ExitStatus, i32> {
    let mut raw = 0;
    loop {
        // SAFETY: `raw` outlives the call.
        if unsafe { libc::waitpid(pid, &mut raw, 0) } != -1 {
            return Ok(ExitStatus::from_raw(raw));
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e.raw_os_error().unwrap_or(libc::ECHILD));
        }
    }
}

impl Process {
    // ponytail: one blocked thread per running child, so a long
    // `run-shell -b` holds a thread for its life. The way up is a single
    // reaper on a kqueue (`EVFILT_PROC`, `NOTE_EXIT`) driven by `AsyncFd`.
    fn start(pid: pid_t, kill_on_drop: bool) -> io::Result<Process> {
        let (tx, exited) = watch::channel(false);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let theirs = Arc::clone(&shared);
        let reaper = move || {
            wait_exited(pid);
            let mut s = lock(&theirs);
            s.exited = true;
            if s.dropped {
                let _ = reap(pid);
            }
            drop(s);
            let _ = tx.send(true);
        };
        let thread = std::thread::Builder::new().name("grofe-reap".into());
        if let Err(e) = thread.stack_size(64 * 1024).spawn(reaper) {
            // SAFETY: a signal to our own unreaped child.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            let _ = reap(pid);
            return Err(e);
        }
        Ok(Process {
            pid,
            kill_on_drop,
            shared,
            exited,
        })
    }

    /// The pid, until the child is reaped.
    pub(super) fn id(&self) -> Option<u32> {
        match lock(&self.shared).status {
            Some(_) => None,
            None => u32::try_from(self.pid).ok(),
        }
    }

    pub(super) async fn wait(&mut self) -> io::Result<ExitStatus> {
        let exited = self.exited.wait_for(|e| *e).await;
        exited.map_err(|_| io::Error::other("child reaper ended"))?;
        let pid = self.pid;
        let status = *lock(&self.shared).status.get_or_insert_with(|| reap(pid));
        status.map_err(io::Error::from_raw_os_error)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let mut s = lock(&self.shared);
        s.dropped = true;
        if s.exited && s.status.is_none() {
            s.status = Some(reap(self.pid));
        } else if !s.exited && self.kill_on_drop {
            // SAFETY: not reaped, so `pid` is still this child.
            unsafe { libc::kill(self.pid, libc::SIGKILL) };
        }
    }
}
