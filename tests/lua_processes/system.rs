//! A2a bullet 1: `grofe.system` env, stdin, on_line, timeout_ms.

use std::fs;

use crate::common::{process_gone, wait_for_file};
use crate::helpers::{opt, run_for_r, server, wait_opt};
use crate::support::{lua, lua_ok};

const SET: &str = "grofe.cmd{'set-option','-g','@r',";

#[test]
fn env_reaches_the_child() {
    let srv = server();
    let code = format!(
        "grofe.system({{'sh','-c','printf %s \"$GL_X\"'}}, function(r) {SET} r.stdout}} end, \
         {{env = {{GL_X = 'bar'}}}})"
    );
    assert_eq!(run_for_r(&srv, &code), "bar");
}

#[test]
fn stdin_is_fed_to_the_child() {
    let srv = server();
    let code =
        format!("grofe.system({{'cat'}}, function(r) {SET} r.stdout}} end, {{stdin = 'hello'}})");
    assert_eq!(run_for_r(&srv, &code), "hello");
}

#[test]
fn stdin_larger_than_the_pipe_buffer() {
    let srv = server();
    let code = format!(
        "grofe.system({{'sh','-c','wc -c'}}, function(r) {SET} (r.stdout:gsub('%s','')) }} end, \
         {{stdin = string.rep('x', 262144)}})"
    );
    assert_eq!(run_for_r(&srv, &code), "262144");
}

fn lines_via(srv: &crate::common::TestServer, printf: &str) -> String {
    let code = format!(
        "local got = {{}} grofe.system({{'sh','-c',\"printf '{printf}'\"}}, \
         function(r) {SET} table.concat(got, '|') .. '#' .. r.code}} end, \
         {{on_line = function(l) got[#got + 1] = l end}})"
    );
    run_for_r(srv, &code)
}

#[test]
fn on_line_streams_each_line() {
    let srv = server();
    assert_eq!(lines_via(&srv, r"a\nb\n"), "a|b#0");
}

#[test]
fn on_line_delivers_the_last_line_without_newline() {
    let srv = server();
    assert_eq!(lines_via(&srv, r"a\nb"), "a|b#0");
}

#[test]
fn non_utf8_output_still_completes() {
    let srv = server();
    let code = format!(
        "grofe.system({{'sh','-c',\"printf '\\\\377\\\\376ok'\"}}, \
         function(r) {SET} r.code .. ':' .. r.stdout:sub(-2)}} end)"
    );
    assert_eq!(run_for_r(&srv, &code), "0:ok");
}

#[test]
fn timeout_over_one_hour_is_an_error() {
    let srv = server();
    let err = lua(
        &srv,
        "grofe.system({'true'}, function() end, {timeout_ms = 3600001})",
    )
    .expect_err("over one hour must be rejected");
    assert!(err.contains("timeout"), "{err}");
    lua_ok(
        &srv,
        &format!("grofe.system({{'true'}}, function() {SET} 'ok'}} end, {{timeout_ms = 3600000}})"),
    );
    assert_eq!(wait_opt(&srv, "@r"), "ok");
}

#[test]
fn timeout_kills_the_whole_process_group() {
    let srv = server();
    let pidfile = srv.home.join("grandchild.pid");
    let code = format!(
        "grofe.system({{'sh','-c','sleep 300 >/dev/null 2>&1 & echo $! > {}; wait'}}, \
         function(r) {SET} 'timed'}} end, {{timeout_ms = 500}})",
        pidfile.display()
    );
    lua_ok(&srv, &code);
    let pid: i32 = loop {
        let text = wait_for_file(&pidfile);
        if let Ok(p) = text.trim().parse() {
            break p;
        }
    };
    assert_eq!(wait_opt(&srv, "@r"), "timed");
    let pid = nix::unistd::Pid::from_raw(pid);
    srv.wait_for(|| process_gone(pid), crate::support::WAIT)
        .unwrap_or_else(|e| panic!("{e}: grandchild {pid} survived the timeout"));
    let _ = fs::remove_file(pidfile);
    assert_eq!(opt(&srv, "@r"), "timed");
}

#[test]
fn timeout_covers_a_descendant_that_holds_the_pipes() {
    let srv = server();
    let pidfile = srv.home.join("holder.pid");
    let code = format!(
        "grofe.system({{'sh','-c','sleep 600 & echo $! > {}; echo hi'}}, \
         function(r) {SET} r.code .. ':' .. r.stdout}} end, {{timeout_ms = 500}})",
        pidfile.display()
    );
    lua_ok(&srv, &code);
    assert_eq!(wait_opt(&srv, "@r"), "-1:hi");
    let text = wait_for_file(&pidfile);
    let pid = nix::unistd::Pid::from_raw(text.trim().parse().expect("pid"));
    srv.wait_for(|| process_gone(pid), crate::support::WAIT)
        .unwrap_or_else(|e| panic!("{e}: {pid} survived the timeout"));
    let _ = fs::remove_file(pidfile);
}

#[test]
fn cancel_stops_on_line() {
    let srv = server();
    lua_ok(
        &srv,
        "j = grofe.system({'sh','-c','while true; do echo x; sleep 0.1; done'}, function() end, \
         {timeout_ms = 60000, on_line = function(l) \
         grofe.cmd{'set-option','-g','-a','@n','x'} end})",
    );
    wait_opt(&srv, "@n");
    lua_ok(&srv, "grofe.cancel(j)");
    let at_cancel = opt(&srv, "@n");
    crate::common::quiet_period(std::time::Duration::from_millis(600));
    // A line already queued may still land; the stream must not go on.
    assert!(
        opt(&srv, "@n").len() <= at_cancel.len() + 2,
        "on_line kept running"
    );
}
