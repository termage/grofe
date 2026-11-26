//! Luau scripting end to end: `init.luau` against a real server, with keys
//! pressed through an attached client. Each test has a private socket and
//! config dir (`tests/common`).

mod common;
#[allow(dead_code)]
#[path = "acceptance/harness.rs"]
mod harness;

use std::fs;
use std::time::{Duration, Instant};

use common::TestServer;
use harness::Harness;

const WAIT: Duration = Duration::from_secs(10);
/// What an attached client sees once the server has turned the mouse on.
const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1006h";

fn ok(srv: &TestServer, args: &[&str]) -> String {
    srv.cmd(args).unwrap_or_else(|e| panic!("{args:?}: {e}"))
}

/// A server whose config dir holds `init`, with session `main` running `cat`.
fn with_init(init: &str) -> TestServer {
    let srv = TestServer::builder().unstarted();
    let dir = srv.config_home.join("grofe");
    fs::create_dir_all(&dir).expect("mkdir config");
    fs::write(dir.join("init.luau"), init).expect("write init.luau");
    ok(&srv, &["start-server"]);
    ok(
        &srv,
        &[
            "new-session",
            "-d",
            "-s",
            "main",
            "-x",
            "80",
            "-y",
            "23",
            "cat",
        ],
    );
    srv
}

fn attach(srv: &TestServer) -> Harness {
    let mut a = Harness::spawn_attach_with_env(srv.command(), "main", &[], 80, 24);
    a.wait_for_output(MOUSE_ON, 0);
    a.wait_for("[main]");
    a
}

fn panes(srv: &TestServer) -> usize {
    ok(srv, &["list-panes", "-t", "main"]).lines().count()
}

#[test]
fn init_binds_a_key_that_runs() {
    let srv = with_init("grofe.bind('prefix', 'g', function() grofe.cmd{'split-window'} end)\n");
    let mut a = attach(&srv);
    assert_eq!(panes(&srv), 1);
    a.send("\x02g");
    srv.wait_for(|| panes(&srv) == 2, WAIT)
        .unwrap_or_else(|e| panic!("{e}; log:\n{}", srv.log()));
}

#[test]
fn status_widget_updates_option() {
    let srv = with_init(
        "grofe.every(100, function() grofe.system({'echo','hi'}, function(r) \
         grofe.cmd{'set-option','-g','@w', (r.stdout:gsub('%s+$',''))} end) end)\n",
    );
    srv.wait_for(
        || {
            srv.cmd(&["display-message", "-p", "#{@w}"])
                .is_ok_and(|s| s.trim() == "hi")
        },
        Duration::from_secs(1),
    )
    .unwrap_or_else(|e| panic!("{e}; log:\n{}", srv.log()));
}

#[test]
fn spinning_key_does_not_freeze() {
    let srv = with_init("grofe.bind('prefix', 'x', function() while true do end end)\n");
    let mut a = attach(&srv);
    a.send("\x02x");
    let t = Instant::now();
    srv.wait_for(
        || {
            srv.cmd(&["show-messages"])
                .is_ok_and(|m| m.contains("budget"))
        },
        WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}; log:\n{}", srv.log()));
    ok(&srv, &["list-sessions"]);
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "took {:?}",
        t.elapsed()
    );
    assert!(ok(&srv, &["show-messages"]).contains("budget"));
}

#[test]
fn reload_replaces_bindings() {
    let srv = with_init("grofe.bind('prefix', 'g', function() grofe.cmd{'split-window'} end)\n");
    let mut a = attach(&srv);
    let init = srv.config_home.join("grofe/init.luau");
    fs::write(
        &init,
        "grofe.bind('prefix', 'h', function() grofe.cmd{'split-window'} end)\n",
    )
    .expect("rewrite init.luau");
    ok(&srv, &["reload"]);
    a.send("\x02g");
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(panes(&srv), 1, "the old key still fires");
    a.send("\x02h");
    srv.wait_for(|| panes(&srv) == 2, WAIT)
        .unwrap_or_else(|e| panic!("{e}; log:\n{}", srv.log()));
}

#[test]
fn lua_verb_returns_values() {
    let srv = TestServer::start();
    assert_eq!(ok(&srv, &["lua", "return 1 + 1"]).trim(), "2");
}

/// A deferring command run from a Lua timer must not leave its pending reply
/// behind for the next client request to pick up.
#[test]
fn deferred_command_from_a_timer_does_not_leak_into_the_next_reply() {
    let srv = with_init(
        "local n = 0\n\
         grofe.every(50, function() n = n + 1 if n == 1 then \
         grofe.cmd{'run-shell','sleep 2'} end end)\n",
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        ok(&srv, &["display-message", "-p", "hello"]).trim(),
        "hello"
    );
}

/// Lua source that runs `body` once, from a timer.
fn once(body: &str) -> String {
    format!("local n = 0\ngrofe.every(50, function() n = n + 1 if n == 1 then {body} end end)\n")
}

fn opt(srv: &TestServer, name: &str) -> String {
    ok(srv, &["display-message", "-p", &format!("#{{{name}}}")])
        .trim()
        .to_string()
}

/// A chain held behind a deferring command: the `if-shell` form reaches the
/// hold (the string form splits on `;` into separate runs).
fn chain(name: &str) -> String {
    format!("grofe.cmd{{'if-shell','-F','1','run-shell \"sleep 0.3\" ; set -g @{name} 1'}}")
}

#[test]
fn commands_chained_after_a_deferring_one_still_run() {
    let srv = with_init(&once(&chain("x")));
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(opt(&srv, "@x"), "", "the chain ran before the shell ended");
    srv.wait_for(|| opt(&srv, "@x") == "1", WAIT)
        .unwrap_or_else(|e| panic!("{e}; log:\n{}", srv.log()));
}

#[test]
fn two_deferring_chains_in_one_callback_both_finish() {
    let srv = with_init(&once(&format!("{} {}", chain("a"), chain("b"))));
    srv.wait_for(|| opt(&srv, "@a") == "1" && opt(&srv, "@b") == "1", WAIT)
        .unwrap_or_else(|e| panic!("{e}; log:\n{}", srv.log()));
}

#[test]
fn late_failures_of_a_timer_job_reach_show_messages() {
    let srv = with_init(
        "local n = 0\n\
         grofe.every(50, function() n = n + 1 if n == 1 then \
         grofe.cmd{'if-shell','-F','1','run-shell \"sleep 0.2\" ; select-window -t nosuchwin'} \
         elseif n == 20 then grofe.cmd{'run-shell','exit 3'} end end)\n",
    );
    std::thread::sleep(Duration::from_millis(2000));
    let msgs = ok(&srv, &["show-messages"]);
    assert!(msgs.contains("nosuchwin"), "no chain failure in {msgs:?}");
    assert!(msgs.contains("returned 3"), "no shell failure in {msgs:?}");
}

#[test]
fn deferred_command_from_an_event_does_not_leak() {
    let srv = with_init(
        "grofe.on('option-changed', function(e) if e.name ~= '@t' then return end \
         grofe.cmd{'run-shell','echo leak; sleep 2'} grofe.cmd{'set','-g','@ran','1'} end)\n",
    );
    // The event fires inside this request, which must not wait for the shell.
    let t = Instant::now();
    assert_eq!(ok(&srv, &["set", "-g", "@t", "1"]).trim(), "");
    assert!(t.elapsed() < Duration::from_secs(1), "set waited");
    // No request in between: one would take a leaked reply itself.
    std::thread::sleep(Duration::from_millis(700));
    let t = Instant::now();
    assert_eq!(
        ok(&srv, &["display-message", "-p", "hello"]).trim(),
        "hello"
    );
    assert!(t.elapsed() < Duration::from_secs(1), "reply was held");
    assert_eq!(opt(&srv, "@ran"), "1");
}

#[test]
fn deferred_command_from_a_system_callback_does_not_leak() {
    let srv = with_init(
        "grofe.system({'sh','-c','sleep 1'}, function(r) \
         grofe.cmd{'run-shell','echo leak; sleep 2'} grofe.cmd{'set','-g','@ran','1'} end)\n",
    );
    std::thread::sleep(Duration::from_millis(1500));
    let t = Instant::now();
    assert_eq!(
        ok(&srv, &["display-message", "-p", "hello"]).trim(),
        "hello"
    );
    assert!(t.elapsed() < Duration::from_secs(1), "reply was held");
    assert_eq!(opt(&srv, "@ran"), "1");
}

#[test]
fn display_menu_from_a_timer_does_not_leak() {
    let srv = with_init("");
    let _a = attach(&srv);
    let client = ok(&srv, &["list-clients", "-F", "#{client_name}"])
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    ok(
        &srv,
        &[
            "lua",
            &format!(
                "local n = 0 grofe.every(50, function() n = n + 1 if n == 1 then \
             grofe.cmd{{'display-menu','-c','{client}','Hi','h','display-message hi'}} end end)"
            ),
        ],
    );
    std::thread::sleep(Duration::from_millis(500));
    let t = Instant::now();
    assert_eq!(
        ok(&srv, &["display-message", "-p", "hello"]).trim(),
        "hello"
    );
    assert!(t.elapsed() < Duration::from_secs(1), "reply was held");
    let msgs = ok(&srv, &["show-messages"]);
    assert!(!msgs.contains("lua:"), "lua error in {msgs:?}");
}

#[test]
fn key_bound_run_shell_output_is_shown() {
    let srv = with_init(
        "grofe.bind('prefix', 'r', function() grofe.cmd{'run-shell','echo bound-out'} end)\n",
    );
    let mut a = attach(&srv);
    a.send("\x02r");
    a.wait_for("bound-out");
}

/// A server whose init runs a deferring command at the top level.
fn init_with_run_shell() -> TestServer {
    let srv = TestServer::builder().unstarted();
    let dir = srv.config_home.join("grofe");
    fs::create_dir_all(&dir).expect("mkdir config");
    fs::write(
        dir.join("init.luau"),
        "grofe.cmd{'run-shell','echo from-init; sleep 3'}\n",
    )
    .expect("write init.luau");
    srv
}

#[test]
fn top_level_run_shell_in_init_does_not_hijack_the_first_reply() {
    let srv = init_with_run_shell();
    let t = Instant::now();
    let out = ok(
        &srv,
        &["new-session", "-d", "-s", "main", "-P", "-F", "#{pane_id}"],
    );
    assert!(
        out.trim().starts_with('%') && !out.contains("from-init"),
        "got {out:?}"
    );
    assert!(t.elapsed() < Duration::from_secs(1), "start waited");
}

#[test]
fn reload_with_a_run_shell_init_gets_its_own_reply() {
    let srv = init_with_run_shell();
    ok(&srv, &["new-session", "-d", "-s", "main", "cat"]);
    let t = Instant::now();
    let out = ok(&srv, &["reload"]);
    assert!(!out.contains("from-init"), "got {out:?}");
    assert!(t.elapsed() < Duration::from_secs(1), "reload waited");
    assert_eq!(
        ok(&srv, &["display-message", "-p", "hello"]).trim(),
        "hello"
    );
}
