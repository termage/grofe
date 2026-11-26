//! A2a bullet 5: `pane-created`, `pane-closed`, pane-scoped `grofe.on`.

use crate::helpers::{new_pane, opt, server, stays_empty, wait_opt};
use crate::support::lua_ok;

const APPEND: &str = "grofe.cmd{'set-option','-g','-a','@r', p.pane .. ','}";

#[test]
fn pane_created_event_carries_the_pane() {
    let srv = server();
    lua_ok(
        &srv,
        &format!("grofe.on('pane-created', function(p) {APPEND} end)"),
    );
    let id = new_pane(&srv);
    assert_eq!(wait_opt(&srv, "@r"), format!("{id},"));
}

#[test]
fn pane_closed_event_carries_the_pane() {
    let srv = server();
    let id = new_pane(&srv);
    lua_ok(
        &srv,
        &format!("grofe.on('pane-closed', function(p) {APPEND} end)"),
    );
    srv.cmd(&["kill-pane", "-t", &id]).expect("kill-pane");
    assert_eq!(wait_opt(&srv, "@r"), format!("{id},"));
}

#[test]
fn pane_scoped_subscription_ignores_other_panes() {
    let srv = server();
    let a = new_pane(&srv);
    let b = new_pane(&srv);
    lua_ok(
        &srv,
        &format!("grofe.on('agent-changed', function(p) {APPEND} end, {{pane = '{a}'}})"),
    );
    srv.cmd(&["agent", "report", "working", "-t", &b, "--ts", "1"])
        .expect("report b");
    stays_empty(&srv, "@r");
    srv.cmd(&["agent", "report", "working", "-t", &a, "--ts", "1"])
        .expect("report a");
    assert_eq!(wait_opt(&srv, "@r"), format!("{a},"));
}

#[test]
fn pane_scoped_subscription_stops_after_the_pane_closes() {
    let srv = server();
    let a = new_pane(&srv);
    let b = new_pane(&srv);
    lua_ok(
        &srv,
        &format!("grofe.on('pane-closed', function(p) {APPEND} end, {{pane = '{a}'}})"),
    );
    srv.cmd(&["kill-pane", "-t", &b]).expect("kill b");
    stays_empty(&srv, "@r");
    srv.cmd(&["kill-pane", "-t", &a]).expect("kill a");
    assert_eq!(wait_opt(&srv, "@r"), format!("{a},"));
    // Later closes must not reach it: exactly one delivery in total.
    let c = new_pane(&srv);
    srv.cmd(&["kill-pane", "-t", &c]).expect("kill c");
    crate::common::quiet_period(std::time::Duration::from_millis(400));
    assert_eq!(opt(&srv, "@r"), format!("{a},"));
}
