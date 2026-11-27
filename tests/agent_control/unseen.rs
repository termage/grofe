//! AC14, first half: `done` stays unseen until a client shows the pane.
//! Exists today; these tests prove it.

use crate::harness::Harness;
use crate::helpers::{WAIT, start};

fn unseen(f: &crate::helpers::Fx, pane: &str) -> String {
    f.srv
        .cmd(&["display-message", "-p", "-t", pane, "#{agent_unseen}"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

#[test]
fn done_in_a_hidden_window_is_unseen_until_a_client_shows_it() {
    let f = start(None);
    let pane = f.agent();
    let window = f
        .srv
        .cmd(&["display-message", "-p", "-t", &pane, "#{window_id}"])
        .expect("window id")
        .trim()
        .to_string();
    let _client = Harness::spawn_attach_with_env(f.srv.command(), "main", &[], 80, 24);
    f.srv
        .wait_for(
            || {
                f.srv
                    .cmd(&["list-clients"])
                    .is_ok_and(|o| !o.trim().is_empty())
            },
            WAIT,
        )
        .expect("client attached");
    f.srv
        .cmd(&["agent", "report", "done", "-t", &pane, "--kind", "fake"])
        .expect("report done");
    assert_eq!(f.state(&pane), "done");
    assert_eq!(unseen(&f, &pane), "1", "the window is not on screen");
    // It stays while nobody looks.
    crate::common::quiet_period(std::time::Duration::from_millis(500));
    assert_eq!(unseen(&f, &pane), "1");
    f.srv
        .cmd(&["select-window", "-t", &window])
        .expect("select-window");
    f.srv
        .wait_for(|| unseen(&f, &pane) == "0", WAIT)
        .unwrap_or_else(|e| panic!("{e}: still unseen"));
}

#[test]
fn done_in_the_shown_pane_is_never_unseen() {
    let f = start(None);
    let _client = Harness::spawn_attach_with_env(f.srv.command(), "main", &[], 80, 24);
    f.srv
        .wait_for(
            || {
                f.srv
                    .cmd(&["list-clients"])
                    .is_ok_and(|o| !o.trim().is_empty())
            },
            WAIT,
        )
        .expect("client attached");
    let pane = f
        .srv
        .cmd(&["display-message", "-p", "-t", "main", "#{pane_id}"])
        .expect("pane id")
        .trim()
        .to_string();
    f.srv
        .cmd(&["agent", "report", "done", "-t", &pane, "--kind", "fake"])
        .expect("report done");
    assert_eq!(unseen(&f, &pane), "0");
}
