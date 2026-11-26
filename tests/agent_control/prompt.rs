//! AC12 and the A2b edge cases: `agent prompt`, with and without `--wait`.

use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::common::{CmdOutput, wait_child};
use crate::helpers::{DETECT, WAIT, lua, opt_r, start};

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
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn agent prompt");
    crate::common::quiet_period(Duration::from_millis(800));
    f.srv.cmd(&["kill-pane", "-t", &pane]).expect("kill-pane");
    let status = wait_child(child, WAIT).expect("agent prompt ends when the pane closes");
    assert_eq!(status.code(), Some(2));
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
