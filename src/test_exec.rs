//! Running a script the test just wrote. Linux refuses to exec a file that
//! any process holds open for writing (ETXTBSY, "Text file busy"), and a
//! test thread's write handle leaks into every child another thread forks
//! while it is open, until that child execs. [`write_script`] closes that
//! race: a child process writes the script, so the test process never holds
//! it open. A file the test process does write itself (a `fs::copy` of a
//! binary) is started through [`retry_busy`], the workaround cargo's own
//! test support uses.
//!
//! One copy for the crate's unit tests (`cfg(test)`) and the integration
//! test targets, which include this file by `#[path]`.

use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// How long a fresh script may stay busy before the error stands.
const BUSY_LIMIT: Duration = Duration::from_secs(5);

/// Write `body` to the new executable `path`, from a child `sh`, so no
/// write handle to it ever exists in this process and it can be run at
/// once. Panics if `path` exists: a script rewritten in place is busy while
/// an earlier run of it still executes, so each one gets a path of its own.
pub fn write_script(path: &Path, body: &str) {
    let written = Command::new("/bin/sh")
        .args(["-c", "set -C; cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            let fed = child.stdin.take().map(|mut i| i.write_all(body.as_bytes()));
            let status = child.wait()?;
            fed.transpose()?;
            Ok(status)
        });
    match written {
        Ok(status) if status.success() => {}
        Ok(status) => panic!("writing script {}: sh {status}", path.display()),
        Err(e) => panic!("writing script {}: {e}", path.display()),
    }
}

/// Copy the program `from` to the new file `to`, keeping its mode, from a
/// child `cp`, for the reason [`write_script`] writes from a child.
pub fn copy_program(from: &Path, to: &Path) {
    let copied = Command::new("/bin/sh")
        .args(["-c", "[ ! -e \"$2\" ] && exec cp \"$1\" \"$2\"", "sh"])
        .args([from, to])
        .status();
    match copied {
        Ok(status) if status.success() => {}
        Ok(status) => panic!("copying {} to {}: {status}", from.display(), to.display()),
        Err(e) => panic!("copying {} to {}: {e}", from.display(), to.display()),
    }
}

/// `run` (a spawn, `output` or `status`), again every 10ms while it fails
/// with ETXTBSY, for at most [`BUSY_LIMIT`]. Any other result is returned.
pub fn retry_busy<T>(mut run: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let deadline = Instant::now() + BUSY_LIMIT;
    loop {
        match run() {
            Err(e)
                if e.kind() == io::ErrorKind::ExecutableFileBusy && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            other => return other,
        }
    }
}
