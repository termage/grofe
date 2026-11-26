//! Callback context: client, pane and cwd of the key press, read with
//! `grofe.context()`. Outside a key press the fields are nil.

use crate::common::TestServer;
use crate::helpers::{assert_api, attach, bind, press, set_r, wait_r};
use crate::support::{WAIT, lua_ok, with_init};

fn server_in(dir: &std::path::Path) -> TestServer {
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
        "-c",
        dir.to_str().expect("utf8"),
        "cat",
    ])
    .expect("new-session");
    srv
}

fn client_name(srv: &TestServer) -> String {
    srv.cmd(&["list-clients", "-F", "#{client_name}"])
        .expect("list-clients")
        .trim()
        .to_string()
}

#[test]
fn key_press_callback_sees_client_pane_and_cwd() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().canonicalize().expect("canonicalize");
    let srv = server_in(&cwd);
    assert_api(&srv, "context");
    bind(&srv, "h", &format!("{SHOW_LOCAL} {}", set_r("s")));
    let mut a = attach(&srv, 100, 30);
    let pane = srv
        .cmd(&["display-message", "-p", "#{pane_id}"])
        .expect("pane")
        .trim()
        .to_string();
    press(&mut a, "h");
    let want = format!("{}|{pane}|{}", client_name(&srv), cwd.display());
    assert_eq!(wait_r(&srv), want);
}

const SHOW_LOCAL: &str = "local c = grofe.context(); \
    local s = tostring(c.client) .. '|' .. tostring(c.pane) .. '|' .. tostring(c.cwd);";

#[test]
fn choose_callback_keeps_the_key_press_context() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().canonicalize().expect("canonicalize");
    let srv = server_in(&cwd);
    assert_api(&srv, "context");
    bind(
        &srv,
        "h",
        &format!(
            "grofe.choose({{'only'}}, function(v) {SHOW_LOCAL} {} end)",
            set_r("s")
        ),
    );
    let mut a = attach(&srv, 100, 30);
    let pane = srv
        .cmd(&["display-message", "-p", "#{pane_id}"])
        .expect("pane")
        .trim()
        .to_string();
    press(&mut a, "h");
    a.wait_for("only");
    a.send("\r");
    let want = format!("{}|{pane}|{}", client_name(&srv), cwd.display());
    assert_eq!(wait_r(&srv), want);
}

#[test]
fn outside_a_key_press_the_fields_are_nil() {
    let srv = crate::helpers::server();
    assert_api(&srv, "context");
    let out = lua_ok(&srv, &format!("{SHOW_LOCAL} return s"));
    assert_eq!(out, "nil|nil|nil");
}

#[test]
fn timer_callback_has_nil_fields() {
    let srv = crate::helpers::server();
    assert_api(&srv, "context");
    lua_ok(
        &srv,
        &format!(
            "local id; id = grofe.every(50, function() {SHOW_LOCAL} {} grofe.cancel(id) end)",
            set_r("s")
        ),
    );
    assert_eq!(wait_r(&srv), "nil|nil|nil");
}

#[test]
fn pack_load_has_nil_fields() {
    let srv = with_init(&format!("{SHOW_LOCAL} {}", set_r("s")));
    // `display-message` (the `opt` reader) needs a session to run in.
    srv.cmd(&[
        "new-session",
        "-d",
        "-s",
        "main",
        "-x",
        "80",
        "-y",
        "24",
        "cat",
    ])
    .expect("new-session");
    srv.wait_for(|| !crate::helpers::opt(&srv, "@r").is_empty(), WAIT)
        .unwrap_or_else(|e| panic!("{e}: init never ran; log:\n{}", srv.log()));
    assert_eq!(crate::helpers::opt(&srv, "@r"), "nil|nil|nil");
}
