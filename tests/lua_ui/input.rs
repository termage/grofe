//! `grofe.input(prompt, fn)`.

use crate::helpers::{
    ESC, STORE_ARG, assert_api, attach, bind, press, r_stays_unset, server, wait_r,
};
use crate::support::{WAIT, lua};

fn bind_input(srv: &crate::common::TestServer) {
    bind(
        srv,
        "k",
        &format!("grofe.input('Your name: ', function(v) {STORE_ARG} end)"),
    );
}

#[test]
fn typed_line_reaches_the_callback() {
    let srv = server();
    bind_input(&srv);
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "k");
    a.wait_for("Your name:");
    a.send("hello world");
    a.send("\r");
    assert_eq!(wait_r(&srv), "[hello world]");
    a.wait_until("prompt closed", WAIT, |t| !t.contains("Your name:"));
}

#[test]
fn unicode_text_round_trips() {
    let srv = server();
    bind_input(&srv);
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "k");
    a.wait_for("Your name:");
    a.send("héllo 日本語 ✓");
    a.send("\r");
    assert_eq!(wait_r(&srv), "[héllo 日本語 ✓]");
}

#[test]
fn backspace_edits_the_line() {
    let srv = server();
    bind_input(&srv);
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "k");
    a.wait_for("Your name:");
    a.send("abcx\x7f");
    a.send("\r");
    assert_eq!(wait_r(&srv), "[abc]");
}

#[test]
fn empty_line_is_an_empty_string_not_nil() {
    let srv = server();
    bind_input(&srv);
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "k");
    a.wait_for("Your name:");
    a.send("\r");
    assert_eq!(wait_r(&srv), "[]");
}

#[test]
fn cancel_gives_nil() {
    let srv = server();
    bind_input(&srv);
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "k");
    a.wait_for("Your name:");
    a.send("typed");
    a.send(ESC);
    assert_eq!(wait_r(&srv), "NIL");
    a.wait_until("prompt closed", WAIT, |t| !t.contains("Your name:"));
}

#[test]
fn nothing_is_called_before_enter() {
    let srv = server();
    bind_input(&srv);
    let mut a = attach(&srv, 100, 30);
    press(&mut a, "k");
    a.wait_for("Your name:");
    a.send("partial");
    r_stays_unset(&srv);
}

#[test]
fn bad_arguments_are_errors() {
    let srv = server();
    assert_api(&srv, "input");
    let e = lua(&srv, "grofe.input(5, function() end)").expect_err("prompt not a string");
    assert!(e.contains("input"), "{e}");
    let e = lua(&srv, "grofe.input('p', 'nope')").expect_err("fn not a function");
    assert!(e.contains("input"), "{e}");
}
