//! tmux 3.7b's stock `m`, `M`, `t`, `*`, `<` and `>` under grofe identity,
//! and renaming a pane: `prefix e` and the pane menu's `Rename`, each
//! through the keys an attached client sends.
//!
//! Each test runs its own server on a private `-S` socket.

mod common;
#[allow(dead_code)]
#[path = "acceptance/harness.rs"]
mod harness;

use std::fs;
use std::time::Duration;

use common::{TestServer, try_wait_until};
use harness::Harness;

const WAIT: Duration = Duration::from_secs(10);
const PREFIX: &str = "\x02";

fn ok(srv: &TestServer, args: &[&str]) -> String {
    srv.cmd(args).unwrap_or_else(|e| panic!("{args:?}: {e}"))
}

fn var(srv: &TestServer, fmt: &str) -> String {
    ok(srv, &["display-message", "-p", fmt]).trim().to_string()
}

fn wait_var(srv: &TestServer, fmt: &str, want: &str) {
    try_wait_until(|| var(srv, fmt) == want, WAIT)
        .unwrap_or_else(|e| panic!("{fmt} never {want:?} (is {:?}): {e}", var(srv, fmt)));
}

/// A grofe-identity server with frames on and session `main` of two
/// panes, `%1` active, and a client attached to it.
fn server() -> (TestServer, Harness) {
    let srv = TestServer::builder().frames(true).unstarted();
    let dir = srv.config_home.join("grofe");
    fs::create_dir_all(&dir).expect("mkdir config");
    fs::write(dir.join("config.toml"), "pane-frames = true\n").expect("write config");
    ok(&srv, &["start-server"]);
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
    ok(&srv, &["select-pane", "-t", "%1", "-T", ""]);
    let mut client = Harness::spawn_attach_with_env(srv.command(), "main", &[], 80, 24);
    client.wait_for("[main]");
    (srv, client)
}

#[test]
fn prefix_e_renames_the_active_pane_and_its_frame_shows_it() {
    let (srv, mut client) = server();
    client.send(format!("{PREFIX}e"));
    std::thread::sleep(Duration::from_millis(300));
    client.send("build\r");
    wait_var(&srv, "#{pane_title}", "build");
    assert_ne!(
        ok(&srv, &["display", "-p", "-t", "%0", "#{pane_title}"]).trim(),
        "build"
    );
    client.wait_until("the title in the frame", WAIT, |s| s.contains("\"build\""));
}

#[test]
fn prefix_greater_than_renames_through_the_pane_menu() {
    let (srv, mut client) = server();
    ok(&srv, &["select-pane", "-t", "%1", "-T", "old "]);
    client.send(format!("{PREFIX}>"));
    client.wait_until("the pane menu", WAIT, |s| s.contains("Rename"));
    assert!(client.text().contains("Float"), "{}", client.text());
    client.send("n");
    std::thread::sleep(Duration::from_millis(300));
    // The prompt starts with the title.
    client.send("logs\r");
    wait_var(&srv, "#{pane_title}", "old logs");
}

/// A right click's Rename renames the pane clicked, not the active one.
#[test]
fn the_right_click_menu_renames_the_pane_clicked() {
    let (srv, mut client) = server();
    ok(&srv, &["select-pane", "-t", "%0", "-T", ""]);
    assert_eq!(var(&srv, "#{pane_id}"), "%1");
    // %1 is active; the press is on %0, at client cell (10, 5).
    std::thread::sleep(Duration::from_millis(400));
    client.send("\x1b[<2;11;6M");
    client.wait_until("the pane menu", WAIT, |s| s.contains("Rename"));
    client.send("n");
    std::thread::sleep(Duration::from_millis(300));
    client.send("left\r");
    let title = || ok(&srv, &["display", "-p", "-t", "%0", "#{pane_title}"]);
    try_wait_until(|| title().trim() == "left", WAIT)
        .unwrap_or_else(|e| panic!("%0 is {:?}: {e}", title()));
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
