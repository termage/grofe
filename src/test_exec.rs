//! Running a script the test just wrote. Linux refuses to exec a file that
//! any process holds open for writing (ETXTBSY, "Text file busy"), and a
//! test thread's write handle leaks into every child another thread forks
//! while it is open, until that child execs. [`write_script`] closes that
//! race: a child process writes the script, so the test process never holds
//! it open, and [`copy_program`] copies a binary the same way. A program a
//! test may still find busy is started through [`retry_busy`], the
//! workaround cargo's own test support uses.
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
    let fail = |why: String| -> ! { panic!("writing script {}: {why}", path.display()) };
    let mut child = Command::new("/bin/sh")
        .args(["-c", "set -C; cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| fail(e.to_string()));
    let fed = child.stdin.take().map(|mut i| i.write_all(body.as_bytes()));
    let out = child
        .wait_with_output()
        .unwrap_or_else(|e| fail(e.to_string()));
    // A refused write (the path exists) closes the pipe too: its status and
    // what sh said come before the broken pipe that follows from it.
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        fail(format!("sh {}: {}", out.status, said.trim()));
    }
    if let Some(Err(e)) = fed {
        fail(e.to_string());
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

#[cfg(test)]
mod tests {
    /// The lib's own run only: the integration targets include this file
    /// too, and each would run it again.
    fn in_lib() -> bool {
        module_path!().starts_with("grofe::")
    }

    /// Writing over an existing script panics with sh's refusal, not with
    /// the broken pipe the body's write then gets (a body past the pipe's
    /// buffer, so that write does fail).
    #[test]
    fn an_existing_script_reports_sh_s_error() {
        if !in_lib() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("s");
        super::write_script(&path, "#!/bin/sh\n");
        let body = format!("#!/bin/sh\n#{}\n", "x".repeat(1 << 20));
        let err = std::panic::catch_unwind(|| super::write_script(&path, &body))
            .expect_err("a second write to the same path panics");
        let msg = err.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(msg.contains("sh exit status"), "{msg}");
        assert!(!msg.contains("Broken pipe"), "{msg}");
    }
}
