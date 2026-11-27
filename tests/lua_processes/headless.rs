//! A2a bullet 3: a `system` job shows as a `working` row in the sidebar and
//! grid while it runs. It is not an agent, and `agent list` omits it.

use crate::harness;
use crate::helpers::{server, stays_empty, wait_opt};
use crate::support::{lua, lua_ok};

/// A client with the sidebar open, where a job's row shows.
fn with_sidebar(srv: &crate::common::TestServer) -> harness::Harness {
    let mut a = harness::Harness::spawn_attach_with_env(srv.command(), "main", &[], 160, 30);
    a.wait_for_output(b"\x1b[2J", 0);
    a.send("\x02");
    a.send("b");
    a
}

/// Waits until `text` is no longer on the client's screen.
fn wait_gone(a: &mut harness::Harness, text: &str) {
    a.wait_until(&format!("{text:?} gone"), crate::support::WAIT, |t| {
        !t.contains(text)
    });
}

#[test]
fn system_job_registers_and_leaves_working() {
    let srv = server();
    let mut a = with_sidebar(&srv);
    lua_ok(
        &srv,
        "grofe.system({'sh','-c','sleep 2'}, function(r) \
         grofe.cmd{'set-option','-g','@r','fin'} end, \
         {agent = {kind = 'triage', label = 'issue 7'}})",
    );
    a.wait_for("triage · issue 7");
    assert!(a.text().contains("working"), "{}", a.text());
    wait_opt(&srv, "@r");
    wait_gone(&mut a, "triage · issue 7");
}

#[test]
fn a_clean_exit_leaves_a_done_entry_whatever_done_min_work_says() {
    let srv = server();
    lua_ok(&srv, &job("quick", "exit 0"));
    srv.wait_for(
        || notes(&srv).contains(" - triage done quick"),
        crate::support::WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: notifications {:?}", notes(&srv)));
}

#[test]
fn a_timed_out_job_says_so_in_its_entry() {
    let srv = server();
    lua_ok(
        &srv,
        "grofe.system({'sleep','30'}, function() end, \
         {timeout_ms = 300, agent = {kind = 'triage', label = 'slow'}})",
    );
    srv.wait_for(
        || notes(&srv).contains(" - triage error slow timed out after 300ms"),
        crate::support::WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: notifications {:?}", notes(&srv)));
}

#[test]
fn a_job_killed_by_a_signal_names_it_in_its_entry() {
    let srv = server();
    lua_ok(
        &srv,
        "grofe.system({'sh','-c','kill -9 $$'}, function() end, \
         {agent = {kind = 'triage', label = 'shot'}})",
    );
    srv.wait_for(
        || notes(&srv).contains(" - triage error shot killed by signal 9"),
        crate::support::WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: notifications {:?}", notes(&srv)));
}

#[test]
fn a_job_that_cannot_start_gives_the_spawn_error_in_its_entry() {
    let srv = server();
    lua_ok(
        &srv,
        "grofe.system({'/nonexistent/prog'}, function() end, \
         {agent = {kind = 'triage', label = 'nostart'}})",
    );
    srv.wait_for(
        || notes(&srv).contains(" - triage error nostart /nonexistent/prog:"),
        crate::support::WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: notifications {:?}", notes(&srv)));
}

#[test]
fn a_job_is_not_listed_by_agent_list() {
    let srv = server();
    lua_ok(&srv, &job("listed", "sleep 3"));
    assert_eq!(srv.cmd(&["agent", "list"]).unwrap_or_default().trim(), "");
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
}

#[test]
fn two_jobs_with_one_label_are_two_rows() {
    let srv = server();
    let mut a = harness::Harness::spawn_attach_with_env(srv.command(), "main", &[], 160, 30);
    a.wait_for_output(b"\x1b[2J", 0);
    for _ in 0..2 {
        lua_ok(&srv, &job("zebrafish-7", "sleep 3"));
    }
    a.send("\x02");
    a.send("b");
    a.wait_for("zebrafish-7");
    crate::common::quiet_period(std::time::Duration::from_millis(300));
    assert_eq!(a.text().matches("zebrafish-7").count(), 2, "{}", a.text());
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
fn two_failing_jobs_with_one_label_give_two_notifications() {
    let srv = server();
    for _ in 0..2 {
        lua_ok(&srv, &job("twin", "exit 3"));
    }
    srv.wait_for(
        || notes(&srv).matches(" - triage error twin exit 3").count() == 2,
        crate::support::WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: notifications {:?}", notes(&srv)));
}

#[test]
fn a_job_raises_no_agent_changed_event() {
    let srv = server();
    lua_ok(
        &srv,
        "grofe.on('agent-changed', function() grofe.cmd{'set-option','-g','@ev','x'} end)",
    );
    lua_ok(
        &srv,
        "grofe.system({'sh','-c','exit 3'}, function() grofe.cmd{'set-option','-g','@r','fin'} end, \
         {agent = {kind = 'triage', label = 'quiet-job'}})",
    );
    wait_opt(&srv, "@r");
    srv.wait_for(
        || notes(&srv).contains(" - triage error quiet-job exit 3"),
        crate::support::WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: notifications {:?}", notes(&srv)));
    stays_empty(&srv, "@ev");
}

#[test]
fn failing_job_notifies_error_then_the_record_is_gone() {
    let srv = server();
    let mut a = with_sidebar(&srv);
    lua_ok(&srv, &job("job-e", "sleep 1; exit 3"));
    a.wait_for("triage · job-e");
    srv.wait_for(
        || notes(&srv).contains(" - triage error job-e exit 3"),
        crate::support::WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: notifications {:?}", notes(&srv)));
    wait_gone(&mut a, "triage · job-e");
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
    // Enter on a job's row does nothing: the grid stays open, with no message.
    a.send("\r");
    crate::common::quiet_period(std::time::Duration::from_millis(500));
    assert!(a.text().contains("agents>"), "{}", a.text());
    assert!(!a.text().contains("find pane"), "{}", a.text());
    a.send("\x1b");
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
    let mut a = with_sidebar(&srv);
    let conf = srv.config_home.join("grofe").join("conf.d");
    std::fs::create_dir_all(&conf).expect("conf.d");
    std::fs::write(conf.join("50-notify.toml"), "[notify]\nenabled = false\n").expect("config");
    srv.cmd(&["reload"]).expect("reload");
    lua_ok(
        &srv,
        "grofe.system({'sh','-c','sleep 1; exit 2'}, \
         function() grofe.cmd{'set-option','-g','@r','fin'} end, \
         {agent = {kind = 'triage', label = 'quiet'}})",
    );
    a.wait_for("triage · quiet");
    wait_opt(&srv, "@r");
    wait_gone(&mut a, "triage · quiet");
    assert_eq!(notes(&srv).trim(), "");
}

#[test]
fn cancel_removes_the_record() {
    let srv = server();
    let mut a = with_sidebar(&srv);
    lua_ok(&srv, &format!("j = {}", job("cancelled", "sleep 30")));
    a.wait_for("triage · cancelled");
    lua_ok(&srv, "grofe.cancel(j)");
    wait_gone(&mut a, "triage · cancelled");
    assert_eq!(notes(&srv).trim(), "");
}

#[test]
fn reload_removes_the_record() {
    let srv = server();
    let mut a = with_sidebar(&srv);
    lua_ok(&srv, &job("reloaded", "sleep 30"));
    a.wait_for("triage · reloaded");
    srv.cmd(&["reload"]).expect("reload");
    wait_gone(&mut a, "triage · reloaded");
}

#[test]
fn a_label_is_sanitised_in_the_entry() {
    let srv = server();
    lua_ok(
        &srv,
        "grofe.system({'sh','-c','exit 3'}, function() end, \
         {agent = {kind = 'triage', label = 'a\\27[31mb#[x]'}})",
    );
    srv.wait_for(
        || notes(&srv).contains(" - triage error "),
        crate::support::WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: notifications {:?}", notes(&srv)));
    let out = notes(&srv);
    assert!(!out.contains('\x1b'), "{out:?}");
    assert!(
        out.contains(" - triage error a[31mb##[x] exit 3"),
        "{out:?}"
    );
}
