//! A2a bullet 3: a `system` job registers as a headless agent record.
//! Assumed shape (open question): `opts.agent = {kind, label, state?}`.

use crate::harness;
use crate::helpers::{server, wait_opt};
use crate::support::{lua, lua_ok};

const LIST: &str = "#{agent_kind}|#{agent_label}|#{agent_state}";

fn list(srv: &crate::common::TestServer) -> String {
    srv.cmd(&["agent", "list", "-F", LIST]).unwrap_or_default()
}

#[test]
fn system_job_registers_and_leaves_working() {
    let srv = server();
    lua_ok(
        &srv,
        "grofe.system({'sh','-c','sleep 2'}, function(r) \
         grofe.cmd{'set-option','-g','@r','fin'} end, \
         {agent = {kind = 'triage', label = 'issue 7'}})",
    );
    srv.wait_for(
        || list(&srv).lines().any(|l| l == "triage|issue 7|working"),
        crate::support::WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: list was {:?}", list(&srv)));
    wait_opt(&srv, "@r");
    srv.wait_for(|| !list(&srv).contains("|working"), crate::support::WAIT)
        .unwrap_or_else(|e| panic!("{e}: still working: {:?}", list(&srv)));
}

#[test]
fn agent_record_without_kind_is_an_error() {
    let srv = server();
    let err = lua(
        &srv,
        "grofe.system({'true'}, function() end, {agent = {label = 'x'}})",
    )
    .expect_err("kind is required");
    assert!(err.contains("kind"), "{err}");
    assert_eq!(list(&srv).trim(), "");
}

#[test]
fn headless_record_shows_in_the_sidebar_until_replaced() {
    let srv = server();
    let mut a = harness::Harness::spawn_attach_with_env(srv.command(), "main", &[], 160, 30);
    a.wait_for_output(b"\x1b[2J", 0);
    lua_ok(
        &srv,
        "grofe.system({'sleep','3'}, function() end, \
         {agent = {kind = 'triage', label = 'zebrafish-7'}})",
    );
    a.send("\x02");
    a.send("b");
    a.wait_for("zebrafish-7");
    // The same label again replaces the record: still one.
    lua_ok(
        &srv,
        "grofe.system({'sleep','3'}, function() end, \
         {agent = {kind = 'triage', label = 'zebrafish-7'}})",
    );
    assert_eq!(list(&srv).lines().count(), 1, "{:?}", list(&srv));
}

fn notes(srv: &crate::common::TestServer) -> String {
    srv.cmd(&["notify", "list"]).unwrap_or_default()
}

fn job(label: &str, script: &str) -> String {
    format!(
        "grofe.system({{'sh','-c','{script}'}}, function() end, \
         {{agent = {{kind = 'triage', label = '{label}'}}}})"
    )
}

#[test]
fn failing_job_notifies_error_then_the_record_is_gone() {
    let srv = server();
    lua_ok(&srv, &job("job-e", "sleep 1; exit 3"));
    srv.wait_for(
        || list(&srv).lines().any(|l| l == "triage|job-e|working"),
        crate::support::WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: list was {:?}", list(&srv)));
    srv.wait_for(
        || notes(&srv).contains("triage error exit 3"),
        crate::support::WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: notifications {:?}", notes(&srv)));
    assert_eq!(list(&srv).trim(), "");
}

#[test]
fn running_record_shows_in_the_grid_and_its_notification_in_the_tray() {
    let srv = server();
    let mut a = harness::Harness::spawn_attach_with_env(srv.command(), "main", &[], 160, 30);
    a.wait_for_output(b"\x1b[2J", 0);
    lua_ok(&srv, &job("grid-job", "sleep 2; exit 4"));
    a.send("\x02");
    a.send("O");
    a.wait_for("grid-job");
    // Enter on a headless row has nothing to jump to, and says nothing.
    a.send("\r");
    crate::common::quiet_period(std::time::Duration::from_millis(500));
    assert!(!a.text().contains("find pane"), "{}", a.text());
    srv.wait_for(|| notes(&srv).contains("error"), crate::support::WAIT)
        .unwrap_or_else(|e| panic!("{e}: notifications {:?}", notes(&srv)));
    a.send("\x02");
    a.send("y");
    a.wait_for("grid-job");
    a.send("\r");
    crate::common::quiet_period(std::time::Duration::from_millis(500));
    assert!(!a.text().contains("find pane"), "{}", a.text());
}

#[test]
fn record_is_gone_after_the_job_with_notifications_off() {
    let srv = server();
    let conf = srv.config_home.join("grofe").join("conf.d");
    std::fs::create_dir_all(&conf).expect("conf.d");
    std::fs::write(conf.join("50-notify.toml"), "[notify]\nenabled = false\n").expect("config");
    srv.cmd(&["reload"]).expect("reload");
    lua_ok(
        &srv,
        "grofe.system({'sh','-c','exit 2'}, \
         function() grofe.cmd{'set-option','-g','@r','fin'} end, \
         {agent = {kind = 'triage', label = 'quiet'}})",
    );
    wait_opt(&srv, "@r");
    assert_eq!(list(&srv).trim(), "");
    assert_eq!(notes(&srv).trim(), "");
}

#[test]
fn cancel_removes_the_record() {
    let srv = server();
    lua_ok(&srv, &format!("j = {}", job("cancelled", "sleep 30")));
    assert_eq!(list(&srv).trim(), "triage|cancelled|working");
    lua_ok(&srv, "grofe.cancel(j)");
    assert_eq!(list(&srv).trim(), "");
    assert_eq!(notes(&srv).trim(), "");
}

#[test]
fn reload_removes_the_record() {
    let srv = server();
    lua_ok(&srv, &job("reloaded", "sleep 30"));
    assert_eq!(list(&srv).trim(), "triage|reloaded|working");
    srv.cmd(&["reload"]).expect("reload");
    assert_eq!(list(&srv).trim(), "");
}
