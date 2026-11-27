//! A server with a session, an attached client, key presses, and small
//! readers. Lua callbacks report through the global option `@r`.

use std::time::Duration;

use crate::common::TestServer;
use crate::harness::Harness;
use crate::support::{WAIT, lua_ok};

pub(crate) const PREFIX: &str = "\x02";
pub(crate) const ESC: &str = "\x1b";
pub(crate) const DOWN: &str = "\x1b[B";

/// A started server with session `main` running `cat`.
pub(crate) fn server() -> TestServer {
    let srv = TestServer::start();
    srv.cmd(&[
        "new-session",
        "-d",
        "-s",
        "main",
        "-x",
        "100",
        "-y",
        "30",
        "cat",
    ])
    .expect("new-session");
    srv
}

/// A client attached to `main`, past its first redraw and known to the server.
pub(crate) fn attach(srv: &TestServer, cols: u16, rows: u16) -> Harness {
    let mut a = Harness::spawn_attach_with_env(srv.command(), "main", &[], cols, rows);
    a.wait_for_output(b"\x1b[2J", 0);
    srv.wait_for(
        || {
            srv.cmd(&["list-clients"])
                .map(|o| !o.trim().is_empty())
                .unwrap_or(false)
        },
        WAIT,
    )
    .expect("client attached");
    a
}

/// Types prefix and `key`.
pub(crate) fn press(a: &mut Harness, key: &str) {
    a.send(PREFIX);
    a.send(key);
}

/// Binds prefix `key` to a Lua body.
pub(crate) fn bind(srv: &TestServer, key: &str, body: &str) {
    lua_ok(
        srv,
        &format!("grofe.bind('prefix', '{key}', function() {body} end)"),
    );
}

/// Lua that stores `expr` (a string expression) in `@r`.
pub(crate) fn set_r(expr: &str) -> String {
    format!("grofe.cmd{{'set-option','-g','@r', {expr}}}")
}

/// Lua that stores a callback argument `v` in `@r`: `NIL`, or `[v]`.
pub(crate) const STORE_ARG: &str =
    "grofe.cmd{'set-option','-g','@r', v == nil and 'NIL' or ('[' .. tostring(v) .. ']')}";

/// The value of global user option `name` ("" when unset).
pub(crate) fn opt(srv: &TestServer, name: &str) -> String {
    srv.cmd(&["display-message", "-p", &format!("#{{{name}}}")])
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Waits until `@r` is set and returns it.
pub(crate) fn wait_r(srv: &TestServer) -> String {
    srv.wait_for(|| !opt(srv, "@r").is_empty(), WAIT)
        .unwrap_or_else(|e| panic!("{e}: @r never set; log:\n{}", srv.log()));
    opt(srv, "@r")
}

/// Asserts `@r` stays unset for a short while.
pub(crate) fn r_stays_unset(srv: &TestServer) {
    crate::common::quiet_period(Duration::from_millis(500));
    assert_eq!(opt(srv, "@r"), "", "@r was set; log:\n{}", srv.log());
}

/// A new detached window running `cmd`; its pane id.
pub(crate) fn new_pane(srv: &TestServer, cmd: &str) -> String {
    srv.cmd(&[
        "new-window",
        "-d",
        "-t",
        "main",
        "-P",
        "-F",
        "#{pane_id}",
        cmd,
    ])
    .expect("new-window")
    .trim()
    .to_string()
}

/// A new detached window with an agent of `state` reported on its pane.
pub(crate) fn agent_pane(srv: &TestServer, cmd: &str, state: &str) -> String {
    let pane = new_pane(srv, cmd);
    srv.cmd(&["agent", "report", state, "-t", &pane, "--kind", "claude"])
        .expect("agent report");
    pane
}

/// Asserts `grofe.<name>` exists, so error tests cannot pass on a missing API.
pub(crate) fn assert_api(srv: &TestServer, name: &str) {
    let kind = lua_ok(srv, &format!("return type(grofe.{name})"));
    assert_eq!(kind, "function", "grofe.{name} is not implemented");
}
