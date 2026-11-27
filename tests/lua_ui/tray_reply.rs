//! Tray reply: Enter on an agent entry that opted in opens `grofe.input`
//! and sends the text to the pane. Entries without it keep the jump.

use std::fs;

use crate::common::quiet_period;
use crate::helpers::{ESC, PREFIX, agent_pane, assert_api, attach, server};
use crate::support::{WAIT, lua_ok};

/// A server whose second window runs `cat > file`, with an agent reported
/// there. Returns the pane, its window id and the file.
fn with_listener(
    dir: &tempfile::TempDir,
) -> (
    crate::common::TestServer,
    String,
    String,
    std::path::PathBuf,
) {
    let srv = server();
    let file = dir.path().join("got.txt");
    let cmd = format!("cat > '{}'", file.display());
    let pane = agent_pane(&srv, &cmd, "idle");
    let window = srv
        .cmd(&["display-message", "-p", "-t", &pane, "#{window_id}"])
        .expect("window id")
        .trim()
        .to_string();
    (srv, pane, window, file)
}

fn active_window(srv: &crate::common::TestServer) -> String {
    srv.cmd(&["display-message", "-p", "#{window_id}"])
        .expect("window id")
        .trim()
        .to_string()
}

fn open_tray(a: &mut crate::harness::Harness) {
    a.send(PREFIX);
    a.send("y");
}

#[test]
fn enter_on_an_opted_in_entry_asks_and_sends_the_reply() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (srv, pane, _w, file) = with_listener(&dir);
    assert_api(&srv, "notify");
    lua_ok(
        &srv,
        &format!("grofe.notify('Need answer', {{pane = '{pane}', reply = true}})"),
    );
    let home = active_window(&srv);
    let mut a = attach(&srv, 100, 30);
    open_tray(&mut a);
    a.wait_for("Need answer");
    a.send("\r");
    a.wait_until("tray closed", WAIT, |t| !t.contains("Need answer"));
    assert_eq!(active_window(&srv), home, "a reply entry does not jump");
    quiet_period(std::time::Duration::from_millis(300));
    a.send("héllo 日本");
    a.send("\r");
    srv.wait_for(
        || fs::read_to_string(&file).is_ok_and(|s| s == "héllo 日本\n"),
        WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: pane got {:?}", fs::read_to_string(&file)));
}

#[test]
fn escape_in_the_reply_sends_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (srv, pane, _w, file) = with_listener(&dir);
    lua_ok(
        &srv,
        &format!("grofe.notify('Need answer', {{pane = '{pane}', reply = true}})"),
    );
    let home = active_window(&srv);
    let mut a = attach(&srv, 100, 30);
    open_tray(&mut a);
    a.wait_for("Need answer");
    a.send("\r");
    a.wait_until("tray closed", WAIT, |t| !t.contains("Need answer"));
    assert_eq!(active_window(&srv), home, "a reply entry does not jump");
    quiet_period(std::time::Duration::from_millis(300));
    a.send("draft");
    a.send(ESC);
    quiet_period(std::time::Duration::from_millis(800));
    assert_eq!(fs::read_to_string(&file).unwrap_or_default(), "");
}

#[test]
fn entry_without_opt_in_jumps_as_before() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (srv, pane, window, file) = with_listener(&dir);
    lua_ok(
        &srv,
        &format!("grofe.notify('Plain entry', {{pane = '{pane}'}})"),
    );
    let mut a = attach(&srv, 100, 30);
    open_tray(&mut a);
    a.wait_for("Plain entry");
    a.send("\r");
    srv.wait_for(
        || {
            srv.cmd(&["display-message", "-p", "#{window_id}"])
                .is_ok_and(|w| w.trim() == window)
        },
        WAIT,
    )
    .expect("jumped to the entry's window");
    quiet_period(std::time::Duration::from_millis(300));
    assert_eq!(
        fs::read_to_string(&file).unwrap_or_default(),
        "",
        "no reply was asked"
    );
}

/// The reply text is untrusted: control characters are stripped on the way
/// to the pane, as for `agent prompt`.
#[test]
fn reply_text_is_sanitised() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (srv, pane, _w, file) = with_listener(&dir);
    lua_ok(
        &srv,
        &format!("grofe.notify('Need answer', {{pane = '{pane}', reply = true}})"),
    );
    let home = active_window(&srv);
    let mut a = attach(&srv, 100, 30);
    open_tray(&mut a);
    a.wait_for("Need answer");
    a.send("\r");
    a.wait_until("tray closed", WAIT, |t| !t.contains("Need answer"));
    assert_eq!(active_window(&srv), home, "a reply entry does not jump");
    quiet_period(std::time::Duration::from_millis(300));
    a.send("a\x16\x07b");
    a.send("\r");
    srv.wait_for(
        || fs::read_to_string(&file).is_ok_and(|s| !s.is_empty()),
        WAIT,
    )
    .expect("reply arrived");
    let got = fs::read_to_string(&file).expect("read");
    assert!(
        !got.chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t'),
        "control characters reached the pane: {got:?}"
    );
}

/// An entry whose pane has closed has nobody to answer: Enter says so and
/// asks for nothing.
#[test]
fn enter_on_a_reply_entry_of_a_closed_pane_says_so_and_asks_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (srv, pane, _w, _file) = with_listener(&dir);
    lua_ok(
        &srv,
        &format!("grofe.notify('Need answer', {{pane = '{pane}', reply = true}})"),
    );
    srv.cmd(&["kill-pane", "-t", &pane]).expect("kill-pane");
    let mut a = attach(&srv, 100, 30);
    open_tray(&mut a);
    a.wait_for("Need answer");
    a.send("\r");
    a.wait_for(&format!("pane {pane} closed"));
    quiet_period(std::time::Duration::from_millis(300));
    assert!(!a.text().contains("Reply to"), "a prompt opened");
}

/// A pane that exists but holds no agent has nobody to answer either.
#[test]
fn enter_on_a_reply_entry_of_a_non_agent_pane_says_so_and_asks_nothing() {
    let srv = server();
    let pane = crate::helpers::new_pane(&srv, "cat");
    lua_ok(
        &srv,
        &format!("grofe.notify('Need answer', {{pane = '{pane}', reply = true}})"),
    );
    let mut a = attach(&srv, 100, 30);
    open_tray(&mut a);
    a.wait_for("Need answer");
    a.send("\r");
    a.wait_for(&format!("pane {pane} is not an agent pane"));
    quiet_period(std::time::Duration::from_millis(300));
    assert!(!a.text().contains("Reply to"), "a prompt opened");
}
