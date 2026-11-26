//! tmux 3.7b's stock `m`, `M`, `t`, `*`, `<` and `>` under grofe identity,
//! and renaming a pane: `prefix e` and the pane menu's `Rename`, each
//! through the keys an attached client sends.
//!
//! Each test runs its own server on a private `-S` socket.

mod common;
#[allow(dead_code)]
#[path = "acceptance/harness.rs"]
mod harness;

use std::time::Duration;

use common::{TestServer, try_wait_until};
use harness::Harness;

const WAIT: Duration = Duration::from_secs(10);
const PREFIX: &str = "\x02";
/// What an attached client sees once the server has turned the mouse on.
const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1006h";
/// The rename prompt, named after its template's command.
const RENAME_PROMPT: &str = "(select-pane)";
/// A name the prompt's answer must keep as typed: no quote, backslash,
/// `$` or `;` in it is read as syntax.
const AWKWARD: &str = r#"a\b "q" it's $HOME ;x"#;

fn ok(srv: &TestServer, args: &[&str]) -> String {
    srv.cmd(args).unwrap_or_else(|e| panic!("{args:?}: {e}"))
}

fn title(srv: &TestServer, pane: &str) -> String {
    ok(srv, &["display-message", "-p", "-t", pane, "#{pane_title}"])
        .trim_end_matches('\n')
        .to_string()
}

fn var(srv: &TestServer, fmt: &str) -> String {
    ok(srv, &["display-message", "-p", fmt]).trim().to_string()
}

fn wait_var(srv: &TestServer, fmt: &str, want: &str) {
    try_wait_until(|| var(srv, fmt) == want, WAIT)
        .unwrap_or_else(|e| panic!("{fmt} never {want:?} (is {:?}): {e}", var(srv, fmt)));
}

fn wait_title(srv: &TestServer, pane: &str, want: &str) {
    try_wait_until(|| title(srv, pane) == want, WAIT)
        .unwrap_or_else(|e| panic!("{pane} never {want:?} (is {:?}): {e}", title(srv, pane)));
}

/// A grofe-identity server with frames on and session `main` of two
/// panes titled `old `, `%1` active, and a client attached to it.
fn server() -> (TestServer, Harness) {
    let srv = TestServer::builder().frames(true).start();
    ok(
        &srv,
        &[
            "new-session",
            "-d",
            "-s",
            "main",
            "-x",
            "80",
            "-y",
            "23",
            "cat",
        ],
    );
    ok(&srv, &["split-window", "-h", "-t", "%0", "cat"]);
    for pane in ["%0", "%1"] {
        ok(&srv, &["select-pane", "-t", pane, "-T", "old "]);
    }
    let mut client = Harness::spawn_attach_with_env(srv.command(), "main", &[], 80, 24);
    client.wait_for_output(MOUSE_ON, 0);
    client.wait_for("[main]");
    (srv, client)
}

/// Types `name` at the rename prompt, once it is open, and answers it.
fn answer(client: &mut Harness, name: &str) {
    client.wait_until("the rename prompt", WAIT, |s| s.contains(RENAME_PROMPT));
    client.send(format!("{name}\r"));
}

/// Opens the active pane's menu with `prefix >` and chooses Rename.
fn menu_rename(client: &mut Harness) {
    client.send(format!("{PREFIX}>"));
    client.wait_until("the pane menu", WAIT, |s| s.contains("Rename"));
    assert!(client.text().contains("Float"), "{}", client.text());
    client.send("n");
}

#[test]
fn prefix_e_renames_the_active_pane_and_its_frame_shows_it() {
    let (srv, mut client) = server();
    client.send(format!("{PREFIX}e"));
    // The prompt starts with the title.
    answer(&mut client, "build");
    wait_title(&srv, "%1", "old build");
    assert_eq!(title(&srv, "%0"), "old ");
    client.wait_until("the title in the frame", WAIT, |s| {
        s.lines().any(|l| l.contains("( 2 ) \"old build\""))
    });
}

#[test]
fn prefix_e_keeps_the_name_as_typed() {
    let (srv, mut client) = server();
    client.send(format!("{PREFIX}e"));
    answer(&mut client, AWKWARD);
    wait_title(&srv, "%1", &format!("old {AWKWARD}"));
}

#[test]
fn prefix_greater_than_renames_through_the_pane_menu() {
    let (srv, mut client) = server();
    menu_rename(&mut client);
    answer(&mut client, "logs");
    wait_title(&srv, "%1", "old logs");
}

#[test]
fn the_pane_menu_keeps_the_name_as_typed() {
    let (srv, mut client) = server();
    menu_rename(&mut client);
    answer(&mut client, AWKWARD);
    wait_title(&srv, "%1", &format!("old {AWKWARD}"));
}

/// A right click's Rename renames the pane clicked, not the active one.
#[test]
fn the_right_click_menu_renames_the_pane_clicked() {
    let (srv, mut client) = server();
    assert_eq!(var(&srv, "#{pane_id}"), "%1");
    // The press is on %0, at client cell (10, 5).
    client.send("\x1b[<2;11;6M");
    client.wait_until("the pane menu", WAIT, |s| s.contains("Rename"));
    client.send("n");
    answer(&mut client, "left");
    wait_title(&srv, "%0", "old left");
    assert_eq!(title(&srv, "%1"), "old ");
}

#[test]
fn prefix_less_than_opens_the_window_menu() {
    let (srv, mut client) = server();
    client.send(format!("{PREFIX}<"));
    client.wait_until("the window menu", WAIT, |s| s.contains("New At End"));
    client.send("W");
    wait_var(&srv, "#{session_windows}", "2");
}

#[test]
fn prefix_m_and_upper_m_mark_and_clear_the_pane() {
    let (srv, mut client) = server();
    client.send(format!("{PREFIX}m"));
    wait_var(&srv, "#{pane_marked}", "1");
    client.send(format!("{PREFIX}M"));
    wait_var(&srv, "#{pane_marked_set}", "0");
}

#[test]
fn prefix_t_shows_a_clock() {
    let (srv, mut client) = server();
    client.send(format!("{PREFIX}t"));
    wait_var(&srv, "#{pane_mode}", "clock-mode");
}

#[test]
fn prefix_star_opens_a_floating_pane() {
    let (srv, mut client) = server();
    client.send(format!("{PREFIX}*"));
    wait_var(&srv, "#{window_panes}", "3");
    wait_var(&srv, "#{pane_floating_flag}", "1");
}
