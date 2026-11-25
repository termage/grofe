//! Sessions, clients and pane commands that several acceptance suites
//! set up the same way.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::common::TestServer;
use crate::harness::Harness;

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1006h";

/// A server with one session `main` running `cmd`, sized for an 80x24
/// client with a status line.
pub fn start(cmd: &str) -> TestServer {
    let srv = TestServer::start();
    srv.cmd(&[
        "new-session",
        "-d",
        "-s",
        "main",
        "-x",
        "80",
        "-y",
        "23",
        cmd,
    ])
    .expect("new-session");
    srv
}

pub fn attach_command(srv: &TestServer, target: &str) -> Command {
    let mut cmd = srv.command();
    cmd.args(["attach", "-t", target])
        .env("TERM", "xterm-256color")
        .env_remove("COLORTERM");
    cmd
}

pub fn attach_mouse(srv: &TestServer) -> Harness {
    let mut a = Harness::spawn_attach_with_env(srv.command(), "main", &[], 80, 24);
    a.wait_for_output(MOUSE_ON, 0);
    a
}

pub fn attach_sized(srv: &TestServer, cols: u16, rows: u16) -> Harness {
    let mut a = Harness::spawn_attach_with_env(srv.command(), "main", &[], cols, rows);
    a.wait_for_output(MOUSE_ON, 0);
    a
}

/// Session `main` on `srv`, running `script` under `sh`, sized for an
/// 80x24 client with a status line.
pub fn new_session(srv: &TestServer, script: &str) {
    let cmd = format!("sh -c {}", shell_quote(script));
    srv.cmd(&[
        "new-session",
        "-d",
        "-s",
        "main",
        "-x",
        "80",
        "-y",
        "23",
        &cmd,
    ])
    .expect("new-session");
}

pub fn wait_flag(flag: &Path) -> String {
    format!(
        "while [ ! -e {} ] && [ -d {} ]; do sleep 0.05; done",
        shell_quote(&flag.to_string_lossy()),
        shell_quote(&flag.parent().unwrap_or(flag).to_string_lossy())
    )
}

/// Reads `n` raw bytes and prints them as hex, so a test sees exactly what
/// reached the pane.
pub fn hex_reader(push: &str, n: usize) -> String {
    format!(
        "printf '{push}ready\\n'; stty raw -echo; \
         b=$(dd bs=1 count={n} 2>/dev/null | od -An -tx1 | tr -d ' \\n'); \
         stty sane; printf 'got:%s:end' \"$b\"; exec sleep 600"
    )
}

pub fn display(srv: &TestServer, target: &str, format: &str) -> String {
    srv.cmd(&["display-message", "-p", "-t", target, format])
        .unwrap_or_default()
        .trim_end()
        .to_string()
}

/// The `[offset/history]` indicator anywhere on screen.
pub fn indicator(text: &str) -> Option<String> {
    text.match_indices('[').find_map(|(i, _)| {
        let rest = &text[i + 1..];
        let end = rest.find(']')?;
        let (a, b) = rest[..end].split_once('/')?;
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        (digits(a) && digits(b)).then(|| format!("[{a}/{b}]"))
    })
}

pub fn pane_var(srv: &TestServer, target: &str, var: &str) -> String {
    srv.cmd(&["display-message", "-p", "-t", target, var])
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Two panes side by side; the right one runs `cmd` and is active.
pub fn side_by_side(left: &str, right: &str) -> (TestServer, String, String) {
    let srv = start(left);
    let left_id = pane_var(&srv, "main", "#{pane_id}");
    srv.cmd(&["split-window", "-h", "-t", "main", right])
        .expect("split");
    let right_id = pane_var(&srv, "main", "#{pane_id}");
    assert_ne!(left_id, right_id);
    (srv, left_id, right_id)
}

pub fn last_row(text: &str) -> &str {
    text.lines().last().unwrap_or_default()
}

const WAIT: Duration = Duration::from_secs(10);

pub fn attach_status(srv: &TestServer) -> Harness {
    let mut a = Harness::spawn_attach_with_env(srv.command(), "main", &[], 80, 24);
    a.wait_until("the status row", WAIT, |text| {
        last_row(text).starts_with("[main]")
    });
    a
}
