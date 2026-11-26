//! AC12, the `agent read` half, and its edge cases.

use crate::helpers::{DETECT, WAIT, lua, opt_r, start};

/// A pane that printed `line 1` .. `line 50` (the last without a newline).
fn printed_lines() -> (crate::helpers::Fx, String) {
    let f = start(Some(DETECT));
    let pane = f.agent();
    f.cue(&pane, "lines");
    f.srv
        .wait_for(|| f.srv.capture(&pane).contains("line 50"), WAIT)
        .expect("the agent printed its lines");
    (f, pane)
}

#[test]
fn read_prints_the_last_n_lines_including_scrollback() {
    let (f, pane) = printed_lines();
    let out = f
        .srv
        .cmd(&["agent", "read", &pane, "-n", "30"])
        .unwrap_or_else(|e| panic!("agent read: {e}; log:\n{}", f.srv.log()));
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.len() <= 30, "{} lines", lines.len());
    assert_eq!(lines.last().map(|l| l.trim_end()), Some("line 50"), "{out}");
    // 30 lines is more than the 24 visible rows, so scrollback is in it.
    assert!(out.contains("line 30\n"), "{out}");
    assert!(!out.contains("line 10\n"), "{out}");
}

#[test]
fn read_without_n_prints_something_ending_at_the_last_line() {
    let (f, pane) = printed_lines();
    let out = f.srv.cmd(&["agent", "read", &pane]).expect("agent read");
    assert!(out.trim_end().ends_with("line 50"), "{out}");
}

#[test]
fn read_n_zero_prints_nothing() {
    let (f, pane) = printed_lines();
    let out = f
        .srv
        .cmd(&["agent", "read", &pane, "-n", "0"])
        .expect("agent read -n 0");
    assert_eq!(out, "");
}

#[test]
fn a_huge_n_is_capped_at_the_pane_history() {
    let (f, pane) = printed_lines();
    let out = f
        .srv
        .cmd(&["agent", "read", &pane, "-n", "1000000"])
        .expect("a huge -n is not an error");
    assert!(out.contains("line 1\n"), "all history is there: {out}");
    assert!(out.trim_end().ends_with("line 50"), "{out}");
    assert!(out.lines().count() < 1000, "capped at the history");
}

#[test]
fn read_takes_the_alternate_screen_when_the_agent_uses_one() {
    let (f, pane) = printed_lines();
    f.cue(&pane, "alt");
    f.srv
        .wait_for(|| f.srv.capture(&pane).contains("alt-screen line"), WAIT)
        .expect("the agent entered its alternate screen");
    let out = f
        .srv
        .cmd(&["agent", "read", &pane, "-n", "5"])
        .expect("agent read");
    assert!(out.contains("alt-screen line"), "{out}");
    assert!(
        !out.contains("line 50"),
        "the main screen is not read: {out}"
    );
}

#[test]
fn lua_read_returns_the_last_lines() {
    let (f, pane) = printed_lines();
    lua(
        &f.srv,
        &format!(
            "local r = grofe.agent.read('{pane}', 3); \
             if type(r) == 'table' then r = table.concat(r, '\\n') end; \
             grofe.cmd{{'set-option', '-g', '@r', (r:gsub('%s+', ' '))}}"
        ),
    );
    f.srv
        .wait_for(|| opt_r(&f.srv).contains("line 50"), WAIT)
        .unwrap_or_else(|e| panic!("{e}: @r = {:?}", opt_r(&f.srv)));
    assert!(!opt_r(&f.srv).contains("line 40"), "{:?}", opt_r(&f.srv));
}
