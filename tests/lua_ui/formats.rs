//! `agent-sidebar-format` and `agent-grid-format`: the row and tile label,
//! with `#{@issue}` readable from the agent's pane.

use crate::helpers::{agent_pane, attach, press, server};
use crate::support::WAIT;

fn tagged(srv: &crate::common::TestServer) -> String {
    let pane = agent_pane(srv, "cat", "working");
    srv.cmd(&["set-option", "-p", "-t", &pane, "@issue", "ENG-42"])
        .expect("tag pane");
    pane
}

fn set_global(srv: &crate::common::TestServer, name: &str, value: &str) -> Result<String, String> {
    srv.cmd(&["set-option", "-g", name, value])
}

#[test]
fn options_exist_with_a_default() {
    let srv = server();
    for name in ["agent-sidebar-format", "agent-grid-format"] {
        let v = srv
            .cmd(&["show-options", "-gv", name])
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!v.trim().is_empty(), "{name} has no default");
    }
}

#[test]
fn sidebar_format_shows_the_pane_issue() {
    let srv = server();
    tagged(&srv);
    set_global(&srv, "agent-sidebar-format", "ROW<#{@issue}>").expect("set format");
    let mut a = attach(&srv, 160, 30);
    press(&mut a, "b");
    a.wait_until("row label", WAIT, |t| t.contains("ROW<ENG-42>"));
}

#[test]
fn grid_format_shows_the_pane_issue() {
    let srv = server();
    tagged(&srv);
    set_global(&srv, "agent-grid-format", "TILE<#{@issue}>").expect("set format");
    let mut a = attach(&srv, 160, 40);
    press(&mut a, "O");
    a.wait_until("tile label", WAIT, |t| t.contains("TILE<ENG-42>"));
}

/// Choice made here: a bad format string is accepted and expands as the
/// format engine does; the sidebar still draws and the server stays up.
#[test]
fn bad_format_string_does_not_break_the_sidebar() {
    let srv = server();
    tagged(&srv);
    set_global(&srv, "agent-sidebar-format", "BAD<#{unterminated").expect("set format");
    let mut a = attach(&srv, 160, 30);
    press(&mut a, "b");
    a.wait_until("sidebar drawn", WAIT, |t| t.contains("agents"));
    assert_eq!(
        srv.cmd(&["display-message", "-p", "up"])
            .map(|s| s.trim().to_string()),
        Ok("up".to_string())
    );
}
