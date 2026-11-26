use std::sync::Arc;

use tokio::sync::mpsc;

use super::*;
use crate::config::Resolved;
use crate::event::Event;
use crate::server::agents::AgentState;
use crate::server::{dispatch, test_ctx, test_state};

// The swappable seam: the kinds come from `agents::kinds::builtin()`
// either way, so SUPERLITE_REAL needs no separate fake here.
fn state_with_pane() -> (State, PaneId) {
    let mut state = test_state();
    let argv = ["new-session", "-d", "-s", "a", "sleep 600"].map(String::from);
    dispatch::run(&mut state, &test_ctx(), &argv).expect("new-session");
    let pane = *state.model.panes.keys().next().expect("a pane");
    (state, pane)
}

fn report(state: ReportState, ts: u64) -> Report {
    Report {
        state,
        kind: Some("claude".into()),
        msg: None,
        blocked: None,
        session_id: None,
        ts,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn report_creates_record() {
    let (mut state, pane) = state_with_pane();
    apply(
        &mut state,
        pane,
        report(ReportState::Set(AgentState::Working), 1),
    );
    let a = state.agents.get(pane).expect("record");
    assert_eq!(a.kind.as_deref(), Some("claude"));
    assert_eq!(a.state, AgentState::Working);
    assert!(a.binding.is_some(), "bound to the foreground process");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_report_that_changes_nothing_keeps_the_source() {
    let (mut state, pane) = state_with_pane();
    apply(
        &mut state,
        pane,
        report(ReportState::Set(AgentState::Done), 1),
    );
    assert_eq!(
        state.agents.get(pane).and_then(|a| a.source.clone()),
        Some(Source::Report)
    );
    apply_osc(
        &mut state,
        pane,
        report(ReportState::Set(AgentState::Done), 2),
    );
    assert_eq!(
        state.agents.get(pane).and_then(|a| a.source.clone()),
        Some(Source::Report),
        "the same state again says nothing new"
    );
    apply_osc(
        &mut state,
        pane,
        report(ReportState::Set(AgentState::Idle), 3),
    );
    assert_eq!(
        state.agents.get(pane).and_then(|a| a.source.clone()),
        Some(Source::Osc)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn report_sets_resume_from_session_id() {
    let (mut state, pane) = state_with_pane();
    let mut r = report(ReportState::Set(AgentState::Done), 1);
    r.session_id = Some("s1".into());
    apply(&mut state, pane, r);
    let resume = state.model.panes[&pane].resume.clone();
    // argv[0] is absolute when the pane's PATH holds `claude`, else bare.
    let resume = resume.expect("a resume");
    assert!(
        resume[0] == "claude" || resume[0].ends_with("/claude"),
        "{resume:?}"
    );
    assert_eq!(resume[1..], ["--resume", "s1"]);
    assert!(state.model.panes[&pane].resume_binding.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_session_id_no_resume() {
    let (mut state, pane) = state_with_pane();
    let mut r = report(ReportState::Set(AgentState::Done), 1);
    r.session_id = Some("a b;rm".into());
    let before = state.model.panes[&pane].resume.clone();
    apply(&mut state, pane, r);
    assert_eq!(state.model.panes[&pane].resume, before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_config_ignores_reports() {
    let (mut state, pane) = state_with_pane();
    let mut resolved = Resolved::default();
    resolved.agents.enabled = false;
    configure(&mut state, &resolved);
    apply(
        &mut state,
        pane,
        report(ReportState::Set(AgentState::Working), 1),
    );
    assert!(state.agents.get(pane).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_emits_change() {
    let (mut state, pane) = state_with_pane();
    let (tx, mut rx) = mpsc::channel(4);
    state
        .events
        .subscribe(vec!["agent-changed".to_string()], None, tx)
        .expect("subscribe");
    apply(
        &mut state,
        pane,
        report(ReportState::Set(AgentState::Blocked), 1),
    );
    state.events.flush(&state.model);
    let event = rx.try_recv().expect("event");
    assert!(matches!(
        event,
        Event::AgentChanged { old: None, new: Some(n), .. } if n == "blocked"
    ));
    // The same report again changes nothing.
    apply(
        &mut state,
        pane,
        report(ReportState::Set(AgentState::Blocked), 2),
    );
    state.events.flush(&state.model);
    assert!(rx.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jump_prefers_own_session_for_linked_window() {
    use crate::server::clients::{self, AttachRequest};
    let (mut state, pane) = state_with_pane();
    let ctx = test_ctx();
    let run = |state: &mut State, words: &[&str]| {
        let argv: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        dispatch::run(state, &ctx, &argv).expect("run")
    };
    run(&mut state, &["new-session", "-d", "-s", "b", "sleep 600"]);
    run(&mut state, &["new-window", "-t", "b", "sleep 600"]);
    let window = state.model.panes[&pane].window;
    let b = state
        .model
        .sessions
        .values()
        .find(|s| s.name == "b")
        .map(|s| s.id)
        .expect("session b");
    run(
        &mut state,
        &["link-window", "-s", &format!("a:{window}"), "-t", "b:9"],
    );
    let first_b = state.model.sessions[&b].windows[0];
    state.model.select_window_in(b, first_b);
    let accepted = clients::attach(
        &mut state,
        AttachRequest {
            ctx: ClientCtx {
                client: None,
                ..test_ctx()
            },
            size: (80, 24),
            term: "xterm".into(),
            caps: Default::default(),
            pid: None,
            target: Some("b".into()),
            detach_others: false,
            nest: false,
            tty: None,
            created: false,
            extras: Default::default(),
        },
    )
    .expect("attach");
    let ctx = ClientCtx {
        client: Some(accepted.id),
        ..test_ctx()
    };
    jump(&mut state, &ctx, pane).expect("jump");
    assert_eq!(
        state.clients[&accepted.id].session, b,
        "stays in its own session"
    );
    assert_eq!(state.model.sessions[&b].active, window);
    assert!(is_seen(&state, pane));
    let _ = Arc::new(());
}

#[test]
fn configure_merges_kinds_over_builtins() {
    let mut state = test_state();
    let mut resolved = Resolved::default();
    let kinds = &mut resolved.agents.kinds;
    kinds.entry("claude".into()).or_default().resume = Some("claude -r {id}".into());
    kinds.entry("pi".into()).or_default().enabled = Some(false);
    kinds.entry("mine".into()).or_default().matches = Some(vec!["m".into()]);
    configure(&mut state, &resolved);
    let find = |n: &str| state.agent_kinds.iter().find(|k| k.name == n);
    assert_eq!(
        find("claude").and_then(|k| k.resume.as_deref()),
        Some("claude -r {id}")
    );
    assert!(find("pi").is_none());
    assert_eq!(
        find("mine").map(|k| k.matches.clone()),
        Some(vec!["m".to_string()])
    );
    assert!(find("codex").is_some());
}

fn attach_unfocused(state: &mut State) -> crate::server::clients::Accepted {
    use crate::server::clients::{self, AttachRequest};
    let accepted = clients::attach(
        state,
        AttachRequest {
            ctx: ClientCtx {
                client: None,
                ..test_ctx()
            },
            size: (80, 24),
            term: "xterm".into(),
            caps: Default::default(),
            pid: None,
            target: Some("a".into()),
            detach_others: false,
            nest: false,
            tty: None,
            created: false,
            extras: Default::default(),
        },
    )
    .expect("attach");
    if let Some(c) = state.clients.get_mut(&accepted.id) {
        c.focused = false;
    }
    accepted
}

fn blocked(msg: &str, ts: u64) -> Report {
    let mut r = report(ReportState::Set(AgentState::Blocked), ts);
    r.msg = Some(msg.into());
    r
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn change_raises_and_marks_dirty() {
    let (mut state, pane) = state_with_pane();
    state.notify.cfg.delivery = "off".into();
    let _accepted = attach_unfocused(&mut state);
    let soon = || Instant::now() + std::time::Duration::from_millis(1200);
    crate::server::clients::tick(
        &mut state,
        Instant::now() + std::time::Duration::from_secs(1),
    );
    assert!(crate::server::clients::deadline(&state).is_none_or(|d| d > soon()));
    apply(&mut state, pane, blocked("perm", 1));
    let items = state.notify.items();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].state, "blocked");
    assert_eq!(items[0].msg.as_deref(), Some("perm"));
    assert!(
        crate::server::clients::deadline(&state).is_some_and(|d| d <= soon()),
        "clients redraw"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_unfocused_sends_osc9() {
    let (mut state, pane) = state_with_pane();
    let mut accepted = attach_unfocused(&mut state);
    state.notify.cfg.delivery = "auto".into();
    apply(&mut state, pane, blocked("perm", 1));
    crate::server::clients::tick(
        &mut state,
        Instant::now() + std::time::Duration::from_secs(1),
    );
    let mut out = Vec::new();
    while let Ok(frame) = accepted.output.try_recv() {
        out.extend(frame);
    }
    let out = String::from_utf8_lossy(&out);
    assert!(
        out.contains("\x1b]9;grofe: claude: a:1.1 blocked: perm\x1b\\"),
        "{out:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_no_client_runs_command() {
    let (mut state, pane) = state_with_pane();
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("out");
    state.notify.cfg.delivery = "auto".into();
    state.notify.cfg.command = Some(
        [
            "sh",
            "-c",
            &format!("printf '%s' \"$0\" > {}", file.display()),
            "{message}",
        ]
        .map(String::from)
        .to_vec(),
    );
    apply(&mut state, pane, blocked("perm", 1));
    for _ in 0..100 {
        if file.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let text = std::fs::read_to_string(&file).expect("command ran");
    assert_eq!(text, "a:1.1 blocked: perm");
}

/// `auto` (spec §6.4): with a client attached and none known unfocused
/// nothing leaves the terminal; once one is unfocused OSC 9 does, and the
/// command never runs while any client is attached.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_with_a_focused_client_delivers_nothing() {
    let (mut state, pane) = state_with_pane();
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("ran");
    state.notify.cfg.delivery = "auto".into();
    state.notify.cfg.skip_when_seen = false;
    state.notify.cfg.min_interval = Duration::ZERO;
    state.notify.cfg.command = Some(
        ["sh", "-c", &format!("touch {}", file.display())]
            .map(String::from)
            .to_vec(),
    );
    let mut accepted = attach_unfocused(&mut state);
    if let Some(c) = state.clients.get_mut(&accepted.id) {
        c.focused = true;
    }
    let mut at = Instant::now();
    let mut out = |state: &mut State, rx: &mut mpsc::Receiver<Vec<u8>>| {
        at += Duration::from_secs(1);
        crate::server::clients::tick(state, at);
        String::from_utf8_lossy(&drain(rx)).into_owned()
    };
    apply(&mut state, pane, blocked("focused", 1));
    assert_eq!(state.notify.items().len(), 1, "raised to the tray");
    assert!(!out(&mut state, &mut accepted.output).contains("\x1b]9;"));

    if let Some(c) = state.clients.get_mut(&accepted.id) {
        c.focused = false;
    }
    apply(
        &mut state,
        pane,
        report(ReportState::Set(AgentState::Working), 2),
    );
    apply(&mut state, pane, blocked("unfocused", 3));
    assert!(
        out(&mut state, &mut accepted.output)
            .contains("\x1b]9;grofe: claude: a:1.1 blocked: unfocused")
    );

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!file.exists(), "the command ran with a client attached");
}

/// Spec §2.4: with `focus-events off` a focus-out report does not make
/// the client known unfocused, so its active pane counts as seen and
/// `auto` sends no OSC 9; with it on, the same client is unfocused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn focus_events_off_counts_an_unfocused_client_as_focused() {
    use crate::server::dispatch::options::Owner;
    let (mut state, pane) = state_with_pane();
    let mut accepted = attach_unfocused(&mut state);
    state.notify.cfg.delivery = "auto".into();
    state.notify.cfg.skip_when_seen = false;
    state
        .model
        .options
        .set(Owner::Global, "focus-events", "off", false)
        .expect("set");
    assert!(is_seen(&state, pane), "unknown focus counts as seen");
    apply(&mut state, pane, blocked("perm", 1));
    assert_eq!(state.notify.items().len(), 1, "raised to the tray");
    crate::server::clients::tick(
        &mut state,
        Instant::now() + std::time::Duration::from_secs(1),
    );
    let out = String::from_utf8_lossy(&drain(&mut accepted.output)).into_owned();
    assert!(!out.contains("\x1b]9;"), "{out:?}");

    state
        .model
        .options
        .set(Owner::Global, "focus-events", "on", false)
        .expect("set");
    assert!(!is_seen(&state, pane), "known unfocused is not seen");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worked_time_from_working_since() {
    let (mut state, pane) = state_with_pane();
    state.notify.cfg.delivery = "off".into();
    state.notify.cfg.done_min_work = std::time::Duration::from_millis(100);
    let set = |s, ts| report(ReportState::Set(s), ts);
    apply(&mut state, pane, set(AgentState::Working, 1));
    apply(&mut state, pane, set(AgentState::Done, 2));
    assert!(state.notify.items().is_empty(), "no work, no done");
    apply(&mut state, pane, set(AgentState::Working, 3));
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    apply(&mut state, pane, set(AgentState::Done, 4));
    assert_eq!(state.notify.items().len(), 1);
}

#[test]
fn configure_merges_kind_worktree() {
    let mut state = test_state();
    let mut resolved = Resolved::default();
    resolved
        .agents
        .kinds
        .entry("claude".into())
        .or_default()
        .worktree = Some("always".into());
    configure(&mut state, &resolved);
    let find = |n: &str| state.agent_kinds.iter().find(|k| k.name == n);
    assert_eq!(
        find("claude").and_then(|k| k.worktree.as_deref()),
        Some("always")
    );
    assert_eq!(find("codex").and_then(|k| k.worktree.clone()), None);
}

/// Agent text with a format job, two status ranges, ESC, BEL and a C1
/// control. `x` is a file only a run command would make.
fn evil(x: &std::path::Path) -> String {
    format!(
        "A\x1b]2;PWN\x07B\u{85}C\x1b[31mD#[range=user|x]E##[range=user|y]F#(touch {})",
        x.display()
    )
}

/// What the sanitised text looks like on a surface that draws it raw.
const SHOWN: &str = "A]2;PWNBC[31mD##[range=user|x]E####[range=user|y]F_(touch";

fn drain(rx: &mut mpsc::Receiver<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        out.extend(frame);
    }
    out
}

/// No byte of the injected controls reaches the client: no ESC before
/// the OSC or SGR text, no BEL, no C1 (UTF-8 `C2 85`).
fn assert_inert(bytes: &[u8], what: &str) {
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    let text = String::from_utf8_lossy(bytes);
    assert!(!has(b"\x1b]2;PWN"), "{what}: OSC reached the client");
    assert!(!has(b"PWN\x07"), "{what}: BEL reached the client");
    assert!(!has(b"\x1b[31mD"), "{what}: SGR reached the client");
    assert!(!has("\u{85}".as_bytes()), "{what}: C1 reached the client");
    assert!(!has(b"#(touch"), "{what}: a format job survived");
    assert!(text.contains("PWNBC[31mD"), "{what}: not drawn: {text:?}");
}

/// Spec §2.7 at every sink an attached client sees: the status line
/// (`#{agent_msg}` runs no job and opens no range), the toast, the
/// picker, the tray, the sidebar and the grid.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_text_is_inert_on_every_surface() {
    let (mut state, p1) = state_with_pane();
    crate::server::register_builtins(&mut state);
    state.notify.cfg.delivery = "off".into();
    // In /tmp, not TMPDIR: a long marker path in the agent text pushes
    // the drawn part off the 160-column status line.
    let dir = tempfile::tempdir_in("/tmp").expect("tempdir");
    let x = dir.path().join("X");
    let run = |state: &mut State, ctx: &ClientCtx, words: &[&str]| {
        let argv: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        dispatch::run(state, ctx, &argv).unwrap_or_else(|e| panic!("{words:?}: {e}"));
    };
    run(
        &mut state,
        &test_ctx(),
        &["new-window", "-t", "a", "sleep 600"],
    );
    let p2 = *state
        .model
        .panes
        .keys()
        .find(|p| **p != p1)
        .expect("second pane");
    for (option, value) in [("status-left", ""), ("status-right-length", "200")] {
        run(
            &mut state,
            &test_ctx(),
            &["set-option", "-g", option, value],
        );
    }
    let mut accepted = {
        use crate::server::clients::{self, AttachRequest};
        clients::attach(
            &mut state,
            AttachRequest {
                ctx: ClientCtx {
                    client: None,
                    ..test_ctx()
                },
                size: (160, 50),
                term: "xterm".into(),
                caps: Default::default(),
                pid: None,
                target: Some("a".into()),
                detach_others: false,
                nest: false,
                tty: None,
                created: false,
                extras: Default::default(),
            },
        )
        .expect("attach")
    };
    let ctx = ClientCtx {
        client: Some(accepted.id),
        ..test_ctx()
    };
    let mut at = Instant::now();
    let mut frame = |state: &mut State, rx: &mut mpsc::Receiver<Vec<u8>>| {
        at += Duration::from_secs(1);
        clients::tick(state, at);
        drain(rx)
    };
    frame(&mut state, &mut accepted.output);
    // p2 is in view (status line); p1 is not, so it toasts.
    apply(&mut state, p2, blocked(&evil(&x), 1));
    apply(&mut state, p1, blocked(&evil(&x), 1));
    let shown = state.agents.get(p1).and_then(|a| a.msg.clone());
    assert!(shown.is_some_and(|m| m.starts_with(SHOWN)));

    let out = frame(&mut state, &mut accepted.output);
    assert_inert(&out, "toast");
    let toast = crate::server::toast::deadline(&state, accepted.id);
    assert!(toast.is_some(), "a toast is up");

    run(
        &mut state,
        &test_ctx(),
        &["set-option", "-g", "status-right", "#{agent_msg}"],
    );
    clients::mark_all_dirty(&mut state);
    let out = frame(&mut state, &mut accepted.output);
    assert_inert(&out, "status line");
    let (rows, _) = clients::drawn_status(&state, accepted.id).expect("status");
    use crate::server::status::RangeKind;
    let user: Vec<_> = rows
        .iter()
        .flat_map(|r| &r.ranges)
        .filter(|r| matches!(r.kind, RangeKind::User(_)))
        .collect();
    assert!(user.is_empty(), "agent text opened a range: {user:?}");
    let line: String = rows
        .iter()
        .flat_map(|r| &r.line.spans)
        .map(|s| s.text.as_str())
        .collect();
    assert!(
        line.contains("D#[range=user|x]E##[range=user|y]F_(touch"),
        "{line:?}"
    );
    for surface in [
        &["choose-agent"][..],
        &["choose-notification"],
        &["agent-sidebar", "on"],
        &["agent-grid"],
    ] {
        run(&mut state, &ctx, surface);
        let out = frame(&mut state, &mut accepted.output);
        assert_inert(&out, &surface.join(" "));
        // Close it so the next one is not drawn under it.
        let id = accepted.id;
        match surface[0] {
            "choose-agent" => crate::server::picker::client_gone(&mut state, id),
            "choose-notification" => crate::server::tray::client_gone(&mut state, id),
            _ => {}
        }
        clients::mark_all_dirty(&mut state);
        frame(&mut state, &mut accepted.output);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!x.exists(), "a job ran agent text");
}

/// Spec §2.7 and §6.4 at the outbound sinks: OSC 9 to an unfocused
/// client carries no control byte, and the notifier gets the text as
/// one argv element that no shell reads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_text_is_inert_in_osc9_and_notifier_argv() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Short marker path: agent text is capped at 200 chars, and a long
    // temp path would cut the probes off.
    let short = std::path::PathBuf::from(format!(
        "/tmp/g{}{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos())
    ));
    std::fs::create_dir_all(&short).expect("short dir");
    let x = short.join("X");
    let (mut state, pane) = state_with_pane();
    let mut accepted = attach_unfocused(&mut state);
    state.notify.cfg.delivery = "auto".into();
    apply(&mut state, pane, blocked(&evil(&x), 1));
    crate::server::clients::tick(&mut state, Instant::now() + Duration::from_secs(1));
    let out = drain(&mut accepted.output);
    let text = String::from_utf8_lossy(&out);
    let start = text.find("\x1b]9;").expect("an OSC 9") + 4;
    let end = start + text[start..].find("\x1b\\").expect("its ST");
    let body = &text[start..end];
    assert!(!body.chars().any(char::is_control), "{body:?}");
    assert!(body.contains(SHOWN), "{body:?}");

    let (mut state, pane) = state_with_pane();
    let file = dir.path().join("argv");
    state.notify.cfg.delivery = "auto".into();
    state.notify.cfg.command = Some(
        [
            "sh",
            "-c",
            &format!("printf '%s' \"$1\" > {}", file.display()),
            "sh",
            "{message}",
        ]
        .map(String::from)
        .to_vec(),
    );
    let msg = format!(
        "{} $(touch {}) `touch {}`",
        evil(&x),
        x.display(),
        x.display()
    );
    apply(&mut state, pane, blocked(&msg, 1));
    // The shell's `>` creates the file before printf writes it: wait on content.
    for _ in 0..100 {
        if std::fs::read_to_string(&file).is_ok_and(|s| !s.is_empty()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let got = std::fs::read_to_string(&file).expect("notifier ran");
    assert!(
        got.starts_with(&format!("a:1.1 blocked: {SHOWN}")),
        "{got:?}"
    );
    assert!(got.contains("$(touch") && got.contains("`touch"), "{got:?}");
    assert!(!got.chars().any(char::is_control), "{got:?}");
    assert!(!x.exists(), "the notifier argv went through a shell");
    let _ = std::fs::remove_dir_all(&short);
}

#[test]
fn a_reporting_cli_binds_to_the_pane_root() {
    let argv = |s: &str| Some(vec![s.to_string(), "agent".to_string()]);
    assert_eq!(bind_pid(50, argv("/usr/bin/grofe").as_deref(), Some(7)), 7);
    assert_eq!(bind_pid(50, argv("tmux").as_deref(), Some(7)), 7);
    assert_eq!(bind_pid(50, argv("claude").as_deref(), Some(7)), 50);
    assert_eq!(bind_pid(50, argv("grofe").as_deref(), None), 50);
    assert_eq!(bind_pid(50, None, Some(7)), 50);
}
