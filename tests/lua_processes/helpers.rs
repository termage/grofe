//! Small helpers: a server with a session, and Lua results read back
//! through global user options.

use std::time::Duration;

use crate::common::TestServer;
use crate::support::{WAIT, lua_ok};

/// A started server with session `main` running `cat`.
pub(crate) fn server() -> TestServer {
    let srv = TestServer::start();
    srv.cmd(&[
        "new-session",
        "-d",
        "-s",
        "main",
        "-x",
        "80",
        "-y",
        "23",
        "cat",
    ])
    .expect("new-session");
    srv
}

/// The value of global user option `name` ("" when unset).
pub(crate) fn opt(srv: &TestServer, name: &str) -> String {
    srv.cmd(&["display-message", "-p", &format!("#{{{name}}}")])
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Waits until `name` is non-empty and returns it.
pub(crate) fn wait_opt(srv: &TestServer, name: &str) -> String {
    srv.wait_for(|| !opt(srv, name).is_empty(), WAIT)
        .unwrap_or_else(|e| panic!("{e}: {name} never set; log:\n{}", srv.log()));
    opt(srv, name)
}

/// Runs a Lua snippet and waits for the option `@r` it should set.
pub(crate) fn run_for_r(srv: &TestServer, code: &str) -> String {
    lua_ok(srv, code);
    wait_opt(srv, "@r")
}

/// Asserts `name` stays empty for a short while.
pub(crate) fn stays_empty(srv: &TestServer, name: &str) {
    crate::common::quiet_period(Duration::from_millis(400));
    assert_eq!(opt(srv, name), "", "{name} was set; log:\n{}", srv.log());
}

/// A new detached window's pane id.
pub(crate) fn new_pane(srv: &TestServer) -> String {
    srv.cmd(&[
        "new-window",
        "-d",
        "-t",
        "main",
        "-P",
        "-F",
        "#{pane_id}",
        "cat",
    ])
    .expect("new-window")
    .trim()
    .to_string()
}
