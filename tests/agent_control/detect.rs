//! AC13: detection files, `agent explain`, and what beats screen detection.

use std::time::Duration;

use crate::common::quiet_period;
use crate::helpers::{DETECT, WAIT, start, write_detection};

#[test]
fn a_user_detection_file_drives_the_screen_state() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    f.cue(&pane, "spin");
    f.wait_state(&pane, "working");
    f.cue(&pane, "ask");
    f.wait_state(&pane, "blocked");
    f.cue(&pane, "ready");
    f.wait_state(&pane, "idle");
}

#[test]
fn working_beats_blocked_when_both_match() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    f.cue(&pane, "both");
    f.wait_state(&pane, "working");
    quiet_period(Duration::from_millis(500));
    assert_eq!(f.state(&pane), "working");
}

#[test]
fn only_the_last_lines_are_matched() {
    let f = start(Some(&format!("lines = 3\n{DETECT}")));
    let pane = f.agent();
    f.cue(&pane, "spin");
    f.wait_state(&pane, "working");
    f.cue(&pane, "ready");
    f.wait_state(&pane, "idle");
    f.cue(&pane, "buried");
    f.srv
        .wait_for(|| f.srv.capture(&pane).contains("filler 5"), WAIT)
        .expect("filler printed");
    quiet_period(Duration::from_millis(600));
    assert_eq!(
        f.state(&pane),
        "idle",
        "the spinner is above the last 3 lines"
    );
}

#[test]
fn explain_names_the_kind_state_source_file_and_rule() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    f.cue(&pane, "ask");
    f.wait_state(&pane, "blocked");
    let out = f.explain(&pane).unwrap_or_else(|e| panic!("explain: {e}"));
    for want in ["fake", "blocked", "screen", "agents/fake.toml", "APPROVE"] {
        assert!(out.contains(want), "{want:?} missing from {out:?}");
    }
}

#[test]
fn explain_of_a_pane_that_is_not_an_agent_is_an_error() {
    let f = start(Some(DETECT));
    let err = f
        .srv
        .cmd(&["agent", "explain", "%0"])
        .expect_err("a shell pane has nothing to explain");
    assert!(
        !err.trim().is_empty() && !err.contains("unknown subcommand"),
        "{err:?}"
    );
}

#[test]
fn a_report_beats_the_screen_and_explain_says_so() {
    let f = start(Some(DETECT));
    let pane = f.agent();
    f.srv
        .cmd(&["agent", "report", "blocked", "-t", &pane, "--kind", "fake"])
        .expect("agent report");
    f.wait_state(&pane, "blocked");
    f.cue(&pane, "spin");
    f.srv
        .wait_for(|| f.srv.capture(&pane).contains("WORKING:"), WAIT)
        .expect("spinner shown");
    quiet_period(Duration::from_millis(600));
    assert_eq!(
        f.state(&pane),
        "blocked",
        "the screen does not override a report"
    );
    let out = f.explain(&pane).expect("explain");
    assert!(out.contains("report"), "{out:?}");
    assert!(!out.contains("screen"), "{out:?}");
}

#[test]
fn a_user_file_replaces_the_built_in_one_for_that_kind() {
    let f = start(None);
    // `claude` ships a file; this one replaces it for the fake script.
    write_detection(&f.srv, "claude", DETECT);
    f.srv.cmd(&["reload"]).expect("reload");
    let pane = f.agent_of("claude");
    f.cue(&pane, "ask");
    f.wait_state(&pane, "blocked");
    let out = f.explain(&pane).expect("explain");
    assert!(out.contains("agents/claude.toml"), "{out:?}");
}

#[test]
fn an_invalid_regex_is_reported_and_not_used() {
    let f = start(Some(
        "working = \"WORKING:\"\nblocked = \"APPROVE(\"\nidle = \"READY>\"\n",
    ));
    let pane = f.agent();
    f.srv
        .wait_for(
            || {
                let m = f.srv.cmd(&["show-messages"]).unwrap_or_default();
                m.contains("fake.toml") && m.contains("blocked")
            },
            WAIT,
        )
        .unwrap_or_else(|e| {
            panic!(
                "{e}: show-messages:\n{}",
                f.srv.cmd(&["show-messages"]).unwrap_or_default()
            )
        });
    // The broken file is not used, so the screen changes nothing: the agent
    // stays as it was while its spinner shows.
    f.cue(&pane, "spin");
    f.srv
        .wait_for(|| f.srv.capture(&pane).contains("WORKING:"), WAIT)
        .expect("spinner shown");
    quiet_period(Duration::from_millis(600));
    assert_eq!(f.state(&pane), "idle");
}

#[test]
fn a_built_in_kind_goes_idle_when_its_working_screen_goes_plain() {
    let f = start(None);
    let pane = f.agent_of("claude");
    f.cue(&pane, "busy");
    f.wait_state(&pane, "working");
    let out = f.srv.run(&[
        "agent",
        "prompt",
        &pane,
        "--wait",
        "--timeout",
        "8",
        "--",
        "ready",
    ]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(out.stdout.trim(), "idle");
    f.wait_state(&pane, "idle");
}
