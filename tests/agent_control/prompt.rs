//! AC12 and the A2b edge cases: `agent prompt`, with and without `--wait`.

use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::common::CmdOutput;
use crate::helpers::{DETECT, WAIT, lua, opt_r, start};

/// Control characters in text that comes from outside: an escape sequence
/// (Shift-Tab), ^C, and a C1 control (U+0085), around a newline and a tab.
const HOSTILE: &str = "a\x1b[Zb\x03c\nd\te\u{85}f";

fn code(out: &CmdOutput) -> Option<i32> {
    out.status.code()
}

#[test]
fn wait_prints_the_state_the_agent_reached() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    let out = f.srv.run(&[
        "agent",
        "prompt",
        &pane,
        "--wait",
        "--until",
        "blocked",
        "--timeout",
        "8",
        "--",
        "ask",
    ]);
    assert_eq!(code(&out), Some(0), "{out:?}");
    assert_eq!(out.stdout.trim(), "blocked");
}

#[test]
fn wait_default_until_returns_when_the_work_ends() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    let t = Instant::now();
    let out = f.srv.run(&[
        "agent",
        "prompt",
        &pane,
        "--wait",
        "--timeout",
        "8",
        "--",
        "work",
    ]);
    assert_eq!(code(&out), Some(0), "{out:?}");
    assert_eq!(out.stdout.trim(), "idle");
    // The agent was idle when asked: the wait is for the state after the text.
    assert!(
        t.elapsed() >= Duration::from_millis(700),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn wait_exits_1_on_timeout() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    let t = Instant::now();
    let out = f.srv.run(&[
        "agent",
        "prompt",
        &pane,
        "--wait",
        "--until",
        "blocked",
        "--timeout",
        "1",
        "--",
        "ready",
    ]);
    assert_eq!(code(&out), Some(1), "{out:?}");
    assert!(
        t.elapsed() >= Duration::from_millis(900),
        "{:?}",
        t.elapsed()
    );
    assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
}

#[test]
fn wait_exits_2_when_the_pane_closes() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    let child = f
        .srv
        .command()
        .args([
            "agent",
            "prompt",
            &pane,
            "--wait",
            "--until",
            "blocked",
            "--timeout",
            "30",
            "--",
            "ready",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent prompt");
    crate::common::quiet_period(Duration::from_millis(800));
    f.srv.cmd(&["kill-pane", "-t", &pane]).expect("kill-pane");
    let out = child.wait_with_output().expect("agent prompt ends");
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(&format!("pane {pane} closed")), "{err}");
    assert!(!err.contains("left"), "{err}");
}

#[test]
fn without_wait_it_types_the_text_and_enter_and_returns() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    let out = f.srv.run(&["agent", "prompt", &pane, "--", "ask"]);
    assert_eq!(code(&out), Some(0), "{out:?}");
    // The fake agent only answers a whole line, so Enter was typed too.
    f.wait_state(&pane, "blocked");
}

#[test]
fn text_with_spaces_and_unicode_arrives_whole() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    f.srv
        .cmd(&["agent", "prompt", &pane, "--", "héllo wörld ✓"])
        .expect("agent prompt");
    f.srv
        .wait_for(|| f.srv.capture(&pane).contains("héllo wörld ✓"), WAIT)
        .unwrap_or_else(|e| panic!("{e}: screen:\n{}", f.srv.capture(&pane)));
}

#[test]
fn prompt_to_a_pane_that_is_not_an_agent_is_an_error() {
    let f = start(Some(DETECT));
    let shell = f
        .srv
        .cmd(&["display-message", "-p", "-t", "main", "#{pane_id}"])
        .expect("display-message")
        .trim()
        .to_string();
    let out = f
        .srv
        .run(&["agent", "prompt", &shell, "--", "typed-by-mistake"]);
    assert_eq!(code(&out), Some(1), "{out:?}");
    assert!(
        out.stderr.contains("agent") && !out.stderr.contains("unknown subcommand"),
        "an error saying the pane is no agent: {out:?}"
    );
    crate::common::quiet_period(Duration::from_millis(300));
    assert!(
        !f.srv.capture(&shell).contains("typed-by-mistake"),
        "nothing is typed into a non-agent pane"
    );
}

#[test]
fn unknown_until_state_is_an_error_naming_it() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    let out = f.srv.run(&[
        "agent",
        "prompt",
        &pane,
        "--wait",
        "--until",
        "idle,sleepy",
        "--",
        "typed-by-mistake",
    ]);
    assert_eq!(code(&out), Some(1), "{out:?}");
    assert!(out.stderr.contains("sleepy"), "{out:?}");
    crate::common::quiet_period(Duration::from_millis(300));
    assert!(
        !f.srv.capture(&pane).contains("typed-by-mistake"),
        "a bad --until types nothing"
    );
}

#[test]
fn lua_prompt_calls_back_with_the_state() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    lua(
        &f.srv,
        &format!(
            "grofe.agent.prompt('{pane}', 'ask', {{['until'] = 'blocked', timeout_ms = 8000}}, \
             function(state) grofe.cmd{{'set-option', '-g', '@r', tostring(state)}} end)"
        ),
    );
    f.srv
        .wait_for(|| opt_r(&f.srv) == "blocked", WAIT)
        .unwrap_or_else(|e| panic!("{e}: @r = {:?}; log:\n{}", opt_r(&f.srv), f.srv.log()));
}

#[test]
fn control_characters_are_stripped_and_a_plain_pane_gets_one_line() {
    let f = start(None);
    let (pane, out) = f.raw_agent(false);
    f.srv
        .cmd(&["agent", "prompt", &pane, "--", HOSTILE])
        .expect("agent prompt");
    let want = b"a[Zbc d ef\r";
    assert_eq!(f.raw_bytes(&out, want.len()), want);
}

#[test]
fn control_characters_are_stripped_and_a_bracketed_pane_gets_one_paste() {
    let f = start(None);
    let (pane, out) = f.raw_agent(true);
    f.srv
        .cmd(&["agent", "prompt", &pane, "--", HOSTILE])
        .expect("agent prompt");
    let want = b"\x1b[200~a[Zbc\nd\tef\x1b[201~\r";
    assert_eq!(f.raw_bytes(&out, want.len()), want);
}

#[test]
fn a_pane_that_takes_no_input_is_an_error_naming_it() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    f.srv
        .cmd(&["select-pane", "-d", "-t", &pane])
        .expect("select-pane -d");
    let out = f
        .srv
        .run(&["agent", "prompt", &pane, "--", "typed-by-mistake"]);
    assert_eq!(code(&out), Some(1), "{out:?}");
    assert!(out.stderr.contains(&pane), "{out:?}");
    crate::common::quiet_period(Duration::from_millis(300));
    assert!(!f.srv.capture(&pane).contains("typed-by-mistake"));
}

#[test]
fn the_text_goes_to_the_target_pane_only() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    let other = f
        .srv
        .cmd(&[
            "split-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &pane,
            "cat",
        ])
        .expect("split-window")
        .trim()
        .to_string();
    f.srv
        .cmd(&["set-option", "-w", "-t", &pane, "synchronize-panes", "on"])
        .expect("synchronize-panes");
    f.srv
        .cmd(&["agent", "prompt", &pane, "--", "only-here"])
        .expect("agent prompt");
    f.srv
        .wait_for(|| f.srv.capture(&pane).contains("only-here"), WAIT)
        .expect("the agent got the text");
    crate::common::quiet_period(Duration::from_millis(300));
    assert!(
        !f.srv.capture(&other).contains("only-here"),
        "a synchronized pane got the text"
    );
}

#[test]
fn wait_exits_2_naming_the_pane_when_the_agent_leaves_it() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    let child = f
        .srv
        .command()
        .args([
            "agent",
            "prompt",
            &pane,
            "--wait",
            "--until",
            "blocked",
            "--timeout",
            "30",
            "--",
            "ready",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent prompt");
    crate::common::quiet_period(Duration::from_millis(800));
    f.srv
        .cmd(&["agent", "report", "clear", "-t", &pane])
        .expect("report clear");
    let out = child.wait_with_output().expect("agent prompt ends");
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(&format!("agent left pane {pane}")), "{err}");
    assert!(!err.contains("closed"), "{err}");
}

/// `lua` code that must fail: its error text.
fn lua_err(f: &crate::helpers::Fx, code: &str) -> String {
    f.srv
        .cmd(&["lua", code])
        .expect_err(&format!("lua {code:?} should fail"))
}

#[test]
fn lua_prompt_to_a_pane_that_is_not_an_agent_errors() {
    let f = start(Some(DETECT));
    let shell = f
        .srv
        .cmd(&["display-message", "-p", "-t", "main", "#{pane_id}"])
        .expect("display-message")
        .trim()
        .to_string();
    let err = lua_err(
        &f,
        &format!("grofe.agent.prompt('{shell}', 'typed-by-mistake', nil, function() end)"),
    );
    assert!(err.contains("not an agent"), "{err}");
    crate::common::quiet_period(Duration::from_millis(300));
    assert!(!f.srv.capture(&shell).contains("typed-by-mistake"));
}

#[test]
fn lua_prompt_with_an_unknown_until_errors() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    let err = lua_err(
        &f,
        &format!(
            "grofe.agent.prompt('{pane}', 'typed-by-mistake', {{['until'] = 'sleepy'}}, function() end)"
        ),
    );
    assert!(err.contains("sleepy"), "{err}");
    crate::common::quiet_period(Duration::from_millis(300));
    assert!(!f.srv.capture(&pane).contains("typed-by-mistake"));
}

#[test]
fn lua_prompt_calls_back_with_nil_on_a_timeout() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    lua(
        &f.srv,
        &format!(
            "grofe.agent.prompt('{pane}', 'ready', {{['until'] = 'blocked', timeout_ms = 200}}, \
             function(state) grofe.cmd{{'set-option', '-g', '@r', tostring(state)}} end)"
        ),
    );
    f.srv
        .wait_for(|| opt_r(&f.srv) == "nil", WAIT)
        .unwrap_or_else(|e| panic!("{e}: @r = {:?}; log:\n{}", opt_r(&f.srv), f.srv.log()));
}

#[test]
fn a_reload_in_the_middle_of_a_wait_never_fires_the_old_callback() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    lua(
        &f.srv,
        &format!(
            "grofe.agent.prompt('{pane}', 'ready', {{['until'] = 'blocked', timeout_ms = 1000}}, \
             function(state) grofe.cmd{{'set-option', '-g', '@r', 'old:' .. tostring(state)}} end)"
        ),
    );
    f.srv.cmd(&["reload"]).expect("reload");
    // Past the timeout, and the state the wait was for.
    f.cue(&pane, "ask");
    crate::common::quiet_period(Duration::from_millis(1800));
    assert_eq!(opt_r(&f.srv), "", "the old callback ran");
}

#[test]
fn a_report_that_only_brings_a_message_does_not_end_a_wait() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    f.srv
        .cmd(&["agent", "report", "blocked", "-t", &pane, "--kind", "fake"])
        .expect("report blocked");
    f.wait_state(&pane, "blocked");
    let child = f
        .srv
        .command()
        .args([
            "agent",
            "prompt",
            &pane,
            "--wait",
            "--until",
            "blocked",
            "--timeout",
            "2",
            "--",
            "ready",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent prompt");
    crate::common::quiet_period(Duration::from_millis(500));
    f.srv
        .cmd(&[
            "agent",
            "report",
            "blocked",
            "-t",
            &pane,
            "--msg",
            "still asking",
        ])
        .expect("report with a message");
    let out = child.wait_with_output().expect("agent prompt ends");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("timed out"));
}
