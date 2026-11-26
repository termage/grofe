//! A2a bullet 4: `agent-changed` carries `msg`; `grofe.agent.history`.
//! Assumed: history returns strings, oldest first.

use crate::helpers::{opt, server, wait_opt};
use crate::support::lua_ok;

fn seed(srv: &crate::common::TestServer) -> String {
    let id = opt_pane(srv);
    for (i, (state, msg)) in [("working", "m1"), ("blocked", "m2"), ("done", "m3")]
        .iter()
        .enumerate()
    {
        let ts = (i + 1).to_string();
        srv.cmd(&[
            "agent", "report", state, "-t", &id, "--ts", &ts, "--msg", msg,
        ])
        .expect("agent report");
    }
    id
}

fn opt_pane(srv: &crate::common::TestServer) -> String {
    srv.cmd(&["display-message", "-p", "-t", "main", "#{pane_id}"])
        .expect("pane id")
        .trim()
        .to_string()
}

fn history(srv: &crate::common::TestServer, id: &str, n: u32) -> String {
    let code = format!(
        "grofe.cmd{{'set-option','-g','@r', 'h:' .. table.concat(grofe.agent.history('{id}', {n}), '|')}}"
    );
    lua_ok(srv, &code);
    srv.wait_for(|| opt(srv, "@r").starts_with("h:"), crate::support::WAIT)
        .expect("history result");
    let r = opt(srv, "@r");
    lua_ok(srv, "grofe.cmd{'set-option','-gu','@r'}");
    r
}

#[test]
fn agent_changed_event_carries_msg() {
    let srv = server();
    let id = opt_pane(&srv);
    lua_ok(
        &srv,
        "grofe.on('agent-changed', function(p) \
         grofe.cmd{'set-option','-g','@r', tostring(p.msg)} end)",
    );
    srv.cmd(&[
        "agent",
        "report",
        "blocked",
        "-t",
        &id,
        "--ts",
        "1",
        "--msg",
        "needs-you",
    ])
    .expect("agent report");
    assert_eq!(wait_opt(&srv, "@r"), "needs-you");
}

#[test]
fn history_returns_the_last_n_messages() {
    let srv = server();
    let id = seed(&srv);
    assert_eq!(history(&srv, &id, 2), "h:m2|m3");
}

#[test]
fn history_zero_is_empty_and_large_n_returns_all() {
    let srv = server();
    let id = seed(&srv);
    // An empty result sets "h:", which is non-empty, so wait_opt works.
    assert_eq!(history(&srv, &id, 0), "h:");
    assert_eq!(history(&srv, &id, 99), "h:m1|m2|m3");
}

#[test]
fn history_with_a_bad_pane_is_an_error() {
    let srv = server();
    for pane in ["%9999", "nope:9.9"] {
        let err = crate::support::lua(&srv, &format!("return grofe.agent.history('{pane}', 3)"))
            .expect_err("no such pane");
        assert!(err.contains("pane") || err.contains("find"), "{err}");
    }
}

#[test]
fn history_takes_any_pane_target() {
    let srv = server();
    seed(&srv);
    assert_eq!(history(&srv, "main", 2), "h:m2|m3");
}

#[test]
fn agent_changed_carries_the_reports_own_msg_not_the_stored_one() {
    let srv = server();
    let id = opt_pane(&srv);
    lua_ok(
        &srv,
        "n = 0; grofe.on('agent-changed', function(p) n = n + 1; \
         grofe.cmd{'set-option','-g','@e' .. n, tostring(p.msg)} end)",
    );
    let report = |args: &[&str]| {
        let mut argv = vec!["agent", "report", "blocked", "-t", &id];
        argv.extend_from_slice(args);
        srv.cmd(&argv).expect("agent report");
    };
    report(&["--ts", "1", "--msg", "X"]);
    assert_eq!(wait_opt(&srv, "@e1"), "X");
    // Same state, no msg, a new reason: the record keeps X, the event has none.
    report(&["--ts", "2", "--blocked", "permission"]);
    assert_eq!(wait_opt(&srv, "@e2"), "nil");
}
