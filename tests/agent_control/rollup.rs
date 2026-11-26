//! AC14, second half: session and window rows of the sidebar show the most
//! urgent state of their panes (blocked > done > working > idle).

use crate::harness::Harness;
use crate::helpers::{Fx, start};

/// A detached window `name` in session `session`, with an agent of `state`
/// reported on its pane.
fn agent_in(f: &Fx, session: &str, name: &str, state: &str) -> String {
    let pane = f
        .srv
        .cmd(&[
            "new-window",
            "-d",
            "-t",
            session,
            "-n",
            name,
            "-P",
            "-F",
            "#{pane_id}",
            "cat",
        ])
        .expect("new-window")
        .trim()
        .to_string();
    f.srv
        .cmd(&["agent", "report", state, "-t", &pane, "--kind", "claude"])
        .expect("agent report");
    pane
}

fn with_sidebar(f: &Fx) -> Harness {
    let mut a = Harness::spawn_attach_with_env(f.srv.command(), "main", &[], 160, 30);
    a.wait_for_output(b"\x1b[2J", 0);
    a.send("\x02");
    a.send("b");
    a
}

/// The screen's lines that name `word` and also a state word.
fn lines_with(text: &str, word: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.contains(word))
        .map(str::to_string)
        .collect()
}

fn says(text: &str, row: &str, state: &str) -> bool {
    lines_with(text, row).iter().any(|l| l.contains(state))
}

#[test]
fn session_and_window_rows_show_their_most_urgent_state() {
    let f = start(None);
    f.srv
        .cmd(&[
            "new-session",
            "-d",
            "-s",
            "zeta",
            "-x",
            "80",
            "-y",
            "24",
            "cat",
        ])
        .expect("new-session");
    agent_in(&f, "zeta", "build", "blocked");
    agent_in(&f, "zeta", "lint", "done");
    agent_in(&f, "zeta", "docs", "idle");
    let mut a = with_sidebar(&f);
    a.wait_until("rows show their state", crate::helpers::WAIT, |t| {
        says(t, "zeta", "blocked") && says(t, "lint", "done") && says(t, "docs", "idle")
    });
    let text = a.text();
    for other in ["done", "working", "idle"] {
        assert!(
            !says(&text, "zeta", other),
            "the session row says only blocked:\n{text}"
        );
    }
}

#[test]
fn rows_rank_done_over_working_over_idle() {
    let f = start(None);
    for (session, states) in [
        ("calm", ["done", "working", "idle"]),
        ("busy", ["working", "idle", "idle"]),
        ("quiet", ["idle", "idle", "idle"]),
    ] {
        f.srv
            .cmd(&[
                "new-session",
                "-d",
                "-s",
                session,
                "-x",
                "80",
                "-y",
                "24",
                "cat",
            ])
            .expect("new-session");
        for (i, state) in states.iter().enumerate() {
            agent_in(&f, session, &format!("w{i}"), state);
        }
    }
    let mut a = with_sidebar(&f);
    a.wait_until("rows show their state", crate::helpers::WAIT, |t| {
        says(t, "calm", "done") && says(t, "busy", "working") && says(t, "quiet", "idle")
    });
    let text = a.text();
    for (row, others) in [
        ("calm", ["working", "idle", "blocked"]),
        ("busy", ["done", "idle", "blocked"]),
        ("quiet", ["done", "working", "blocked"]),
    ] {
        for other in others {
            assert!(!says(&text, row, other), "{row} row says {other}:\n{text}");
        }
    }
}

#[test]
fn the_format_roll_up_already_follows_blocked() {
    let f = start(None);
    agent_in(&f, "main", "a", "idle");
    let b = agent_in(&f, "main", "b", "blocked");
    let session = f
        .srv
        .cmd(&["display-message", "-p", "-t", &b, "#{session_agent_state}"])
        .expect("display-message");
    assert_eq!(session.trim(), "blocked");
    let window = f
        .srv
        .cmd(&["display-message", "-p", "-t", &b, "#{window_agent_state}"])
        .expect("display-message");
    assert_eq!(window.trim(), "blocked");
}
