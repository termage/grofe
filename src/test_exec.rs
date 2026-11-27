//! Running a script the test just wrote. Linux refuses to exec a file that
//! any process holds open for writing (ETXTBSY, "Text file busy"), and the
//! write handle of one test thread leaks into every child another thread
//! forks at that moment, until that child execs. So a fresh script can be
//! busy for a moment: write it once, at a path of its own, with
//! [`write_script`], and start it through [`retry_busy`], the workaround
//! cargo's own test support uses.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

/// How long a fresh script may stay busy before the error stands.
const BUSY_LIMIT: Duration = Duration::from_secs(5);

/// Write `body` to the new executable `path`. Panics if `path` exists: a
/// script rewritten in place can be busy, so each one gets a path of its own.
pub fn write_script(path: &Path, body: &str) {
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| {
            file.write_all(body.as_bytes())?;
            file.set_permissions(fs::Permissions::from_mode(0o755))
        });
    if let Err(e) = written {
        panic!("writing script {}: {e}", path.display());
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
