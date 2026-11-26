//! `pane-closed` fires once for every pane that leaves the server, whatever
//! removed it, and never for a pane that only moves. Read through a control
//! `subscribe` connection.

use std::time::Duration;

use crate::common::{Subscription, TestServer};
use crate::helpers::{new_pane, server};
use crate::support::WAIT;
use grofe::event::Event;

const QUIET: Duration = Duration::from_millis(500);

fn ok(srv: &TestServer, args: &[&str]) -> String {
    srv.cmd(args)
        .unwrap_or_else(|e| panic!("{args:?}: {e}\nlog:\n{}", srv.log()))
        .trim()
        .to_string()
}

fn watch(srv: &TestServer) -> Subscription {
    srv.subscribe(&["pane-closed"]).expect("subscribe")
}

/// The panes of every `pane-closed` that arrives (waiting for the first, then
/// until the stream goes quiet), sorted.
fn closed(sub: &mut Subscription) -> Vec<String> {
    let mut out = Vec::new();
    let mut wait = WAIT;
    while let Ok(event) = sub.next(wait) {
        if let Event::PaneClosed { pane } = event {
            out.push(pane.to_string());
        }
        wait = QUIET;
    }
    out.sort();
    out
}

fn none(sub: &mut Subscription) {
    if let Ok(event) = sub.next(QUIET) {
        panic!("unexpected {event:?}");
    }
}

fn sorted(mut ids: Vec<&str>) -> Vec<String> {
    ids.sort();
    ids.into_iter().map(str::to_string).collect()
}

/// A detached split of `target`'s window.
fn split(srv: &TestServer, target: &str) -> String {
    ok(
        srv,
        &[
            "split-window",
            "-d",
            "-t",
            target,
            "-P",
            "-F",
            "#{pane_id}",
            "cat",
        ],
    )
}

/// A scratchpad in `main` running `cmd`, shown; its pane id.
fn pad(srv: &TestServer, cmd: &str) -> String {
    ok(srv, &["new-scratchpad", "-t", "main", cmd]);
    ok(srv, &["display-message", "-p", "-t", "main", "#{pane_id}"])
}

#[test]
fn kill_pane() {
    let srv = server();
    let a = new_pane(&srv);
    let b = split(&srv, &a);
    let mut sub = watch(&srv);
    ok(&srv, &["kill-pane", "-t", &b]);
    assert_eq!(closed(&mut sub), [b]);
}

#[test]
fn kill_window() {
    let srv = server();
    let a = new_pane(&srv);
    let b = split(&srv, &a);
    let mut sub = watch(&srv);
    ok(&srv, &["kill-window", "-t", &a]);
    assert_eq!(closed(&mut sub), sorted(vec![&a, &b]));
}

#[test]
fn kill_session() {
    let srv = server();
    ok(&srv, &["new-session", "-d", "-s", "other", "cat"]);
    let a = ok(
        &srv,
        &["display-message", "-p", "-t", "other", "#{pane_id}"],
    );
    let b = split(&srv, &a);
    let mut sub = watch(&srv);
    ok(&srv, &["kill-session", "-t", "other"]);
    assert_eq!(closed(&mut sub), sorted(vec![&a, &b]));
}

#[test]
fn process_exit() {
    let srv = server();
    ok(&srv, &["set", "-g", "remain-on-exit", "off"]);
    let mut sub = watch(&srv);
    let id = ok(
        &srv,
        &[
            "new-window",
            "-d",
            "-t",
            "main",
            "-P",
            "-F",
            "#{pane_id}",
            "sleep 1",
        ],
    );
    assert_eq!(closed(&mut sub), [id]);
}

#[test]
fn respawn_window_kill_closes_the_extra_panes_only() {
    let srv = server();
    let a = new_pane(&srv);
    let b = split(&srv, &a);
    let mut sub = watch(&srv);
    ok(&srv, &["respawn-window", "-k", "-t", &a]);
    let gone = closed(&mut sub);
    assert_eq!(gone.len(), 1, "{gone:?}");
    assert!(gone[0] == a || gone[0] == b, "{gone:?}");
}

#[test]
fn scratchpad_history_eviction() {
    let srv = server();
    ok(&srv, &["set", "-g", "scratchpad-history", "1"]);
    let mut sub = watch(&srv);
    let first = pad(&srv, "sleep 60");
    ok(&srv, &["toggle-scratchpad", "-t", &first]);
    none(&mut sub);
    let second = pad(&srv, "sleep 61");
    ok(&srv, &["toggle-scratchpad", "-t", &second]);
    assert_eq!(closed(&mut sub), [first]);
}

#[test]
fn hidden_scratchpad_exiting() {
    let srv = server();
    let mut sub = watch(&srv);
    let id = pad(&srv, "sleep 1");
    ok(&srv, &["toggle-scratchpad", "-t", &id]);
    assert_eq!(closed(&mut sub), [id]);
}

#[test]
fn dead_scratchpad_hidden() {
    let srv = server();
    ok(&srv, &["set", "-g", "remain-on-exit", "on"]);
    let id = pad(&srv, "sleep 1");
    let mut sub = watch(&srv);
    srv.wait_for(
        || ok(&srv, &["display-message", "-p", "-t", &id, "#{pane_dead}"]) == "1",
        WAIT,
    )
    .expect("pane dies");
    none(&mut sub);
    ok(&srv, &["toggle-scratchpad", "-t", &id]);
    assert_eq!(closed(&mut sub), [id]);
}

#[test]
fn kill_server_closes_every_pane_hidden_scratchpads_included() {
    let srv = server();
    let first = ok(&srv, &["display-message", "-p", "-t", "main", "#{pane_id}"]);
    let a = new_pane(&srv);
    let hidden = pad(&srv, "sleep 60");
    ok(&srv, &["toggle-scratchpad", "-t", &hidden]);
    let shown = pad(&srv, "sleep 61");
    let mut sub = watch(&srv);
    let _ = srv.cmd(&["kill-server"]);
    let all = sorted(vec![&a, &hidden, &shown, &first]);
    assert_eq!(closed(&mut sub), all);
}

#[test]
fn moving_a_pane_closes_nothing() {
    let srv = server();
    let a = new_pane(&srv);
    let b = new_pane(&srv);
    let c = new_pane(&srv);
    let mut sub = watch(&srv);
    ok(&srv, &["break-pane", "-d", "-s", &split(&srv, &a)]);
    ok(&srv, &["join-pane", "-d", "-s", &b, "-t", &a]);
    ok(&srv, &["move-pane", "-d", "-s", &c, "-t", &a]);
    ok(&srv, &["swap-pane", "-d", "-s", &a, "-t", &b]);
    ok(&srv, &["respawn-pane", "-k", "-t", &a]);
    none(&mut sub);
}
