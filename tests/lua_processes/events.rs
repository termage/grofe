//! A2a bullet 5: `pane-created`, `pane-closed`, pane-scoped `grofe.on`.

use crate::helpers::{new_pane, opt, server, stays_empty, wait_opt};
use crate::support::{lua, lua_ok};

const APPEND: &str = "grofe.cmd{'set-option','-g','-a','@r', p.pane .. ','}";

#[test]
fn empty_pane_scope_is_rejected() {
    let srv = server();
    let err = lua(&srv, "grofe.on('pane-closed', function() end, {pane = ''})")
        .expect_err("an empty target is not a pane");
    assert!(err.contains("is not a pane target"), "{err}");
}

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

#[test]
fn pane_scoped_subscription_to_an_unknown_pane_is_an_error() {
    let srv = server();
    let err = crate::support::lua(
        &srv,
        "grofe.on('pane-closed', function() end, {pane = '%9999'})",
    )
    .expect_err("no such pane");
    assert!(err.contains("pane"), "{err}");
}

#[test]
fn a_command_in_the_subscribing_chunk_is_reported_once() {
    let srv = server();
    lua_ok(
        &srv,
        &format!(
            "grofe.on('pane-created', function(p) {APPEND} end) \
             grofe.cmd{{'new-window','-d','-t','main','cat'}}"
        ),
    );
    let got = wait_opt(&srv, "@r");
    crate::common::quiet_period(std::time::Duration::from_millis(400));
    assert_eq!(opt(&srv, "@r"), got);
    assert_eq!(got.matches(',').count(), 1, "{got}");
}

#[test]
fn a_pane_created_and_killed_in_one_chunk_fires_both() {
    let srv = server();
    lua_ok(
        &srv,
        "grofe.on('pane-created', function(p) \
            grofe.cmd{'set-option','-g','-a','@r','c' .. p.pane .. ','} end) \
         grofe.on('pane-closed', function(p) \
            grofe.cmd{'set-option','-g','-a','@x','x' .. p.pane .. ','} end) \
         local id = grofe.cmd{'new-window','-d','-t','main','-P','-F','#{pane_id}','cat'} \
         grofe.cmd{'kill-pane','-t',(id:gsub('%s', ''))}",
    );
    let made = wait_opt(&srv, "@r");
    let id = made.trim_start_matches('c').trim_end_matches(',');
    assert!(id.starts_with('%'), "{made}");
    assert_eq!(wait_opt(&srv, "@x"), format!("x{id},"));
}

#[test]
fn a_session_or_window_target_is_not_a_pane_scope() {
    let srv = server();
    for target in ["main", "main:", "@0"] {
        let err = crate::support::lua(
            &srv,
            &format!("grofe.on('pane-closed', function() end, {{pane = '{target}'}})"),
        )
        .expect_err(target);
        assert!(err.contains("not a pane target"), "{target}: {err}");
    }
}

#[test]
fn an_unknown_option_key_is_an_error_naming_it() {
    let srv = server();
    let err = crate::support::lua(
        &srv,
        "grofe.on('pane-closed', function() end, {pnae = '%0'})",
    )
    .expect_err("unknown key");
    assert!(err.contains("unknown option pnae"), "{err}");
}

#[test]
fn a_hidden_scratchpad_is_alive_for_a_pane_scope() {
    let srv = server();
    srv.cmd(&["new-scratchpad", "-t", "main", "sleep 60"])
        .expect("scratchpad");
    let pad = srv
        .cmd(&["display-message", "-p", "-t", "main", "#{pane_id}"])
        .expect("pad id")
        .trim()
        .to_string();
    lua_ok(
        &srv,
        &format!("grofe.on('pane-closed', function(p) {APPEND} end, {{pane = '{pad}'}})"),
    );
    srv.cmd(&["toggle-scratchpad", "-t", &pad]).expect("hide");
    stays_empty(&srv, "@r");
    srv.cmd(&["show-scratchpad", "-t", "main:", &pad])
        .expect("show");
    stays_empty(&srv, "@r");
    srv.cmd(&["kill-pane", "-t", &pad]).expect("kill");
    assert_eq!(wait_opt(&srv, "@r"), format!("{pad},"));
    let other = new_pane(&srv);
    srv.cmd(&["kill-pane", "-t", &other]).expect("kill other");
    crate::common::quiet_period(std::time::Duration::from_millis(400));
    assert_eq!(opt(&srv, "@r"), format!("{pad},"));
}
