//! A2a bullet 3: a `system` job registers as a headless agent record.
//! Assumed shape (open question): `opts.agent = {kind, label, state?}`.

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
