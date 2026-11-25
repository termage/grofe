use super::*;
use crate::server::input::{Binding, BindingSource, parse_key};
use crate::server::render::{Overlay, OverlayCtx, Screen};

fn key(name: &str) -> Key {
    parse_key(name).expect("key")
}

fn bind(t: &mut KeyTables, table: &str, k: &str, cmd: &[&str], group: Option<&str>, repeat: bool) {
    let argv = cmd.iter().map(|s| s.to_string()).collect();
    let mut b = Binding::new(vec![argv], repeat, BindingSource::Default);
    b.group = group.map(str::to_string);
    t.insert(table, key(k), b);
}

fn tables() -> KeyTables {
    let mut t = KeyTables::default();
    bind(
        &mut t,
        "prefix",
        "c",
        &["new-window"],
        Some("window"),
        false,
    );
    bind(&mut t, "prefix", "x", &["kill-pane"], Some("pane"), false);
    bind(&mut t, "prefix", "z", &["resize-pane", "-Z"], None, false);
    t
}

fn rows(t: &KeyTables) -> Vec<Row> {
    build_rows(t, "prefix", &|s| s.to_string())
}

fn press_step(ui: &mut ListState, t: &KeyTables, k: &str) -> Step {
    step(ui, &rows(t), t, "prefix", &key(k))
}

#[test]
fn filter_ranks_by_description() {
    let mut t = tables();
    bind(&mut t, "prefix", "b", &["break-pane"], Some("pane"), false);
    t.insert("prefix", key("b"), {
        let mut b = Binding::new(
            vec![vec!["break-pane".into()]],
            false,
            BindingSource::Default,
        );
        b.group = Some("pane".into());
        b.desc = Some("Break pane to a new window".into());
        b
    });
    let mut n = Binding::new(
        vec![vec!["new-window".into()]],
        false,
        BindingSource::Default,
    );
    n.group = Some("window".into());
    n.desc = Some("Create a new window".into());
    t.insert("prefix", key("c"), n);
    let r = rows(&t);
    let mut ui = ListState::default();
    press_step(&mut ui, &t, "C-f");
    for c in ["n", "e", "w"] {
        press_step(&mut ui, &t, c);
    }
    let shown = visible(&r, &ui.query);
    assert_eq!(r[shown[0]].key, key("c"), "contiguous and earliest first");
    assert_eq!(press_step(&mut ui, &t, "Enter"), Step::Run(key("c")));
}

#[test]
fn bound_key_runs_and_closes() {
    let t = tables();
    let mut ui = ListState::default();
    assert_eq!(press_step(&mut ui, &t, "c"), Step::Run(key("c")));
}

#[test]
fn reserved_keys_win() {
    let mut t = tables();
    bind(
        &mut t,
        "prefix",
        "C-n",
        &["next-window"],
        Some("window"),
        false,
    );
    let mut ui = ListState::default();
    assert_eq!(press_step(&mut ui, &t, "C-n"), Step::Redraw);
    assert_eq!(ui.selected, 1);
    assert_eq!(press_step(&mut ui, &t, "Escape"), Step::Close);
    assert_eq!(press_step(&mut ui, &t, "C-c"), Step::Close);
    assert_eq!(press_step(&mut ui, &t, "C-f"), Step::Redraw);
    assert!(ui.filtering);
}

#[test]
fn filter_then_enter_runs_selected() {
    let t = tables();
    let mut ui = ListState::default();
    press_step(&mut ui, &t, "C-f");
    for c in ["w", "i", "n"] {
        assert_eq!(press_step(&mut ui, &t, c), Step::Redraw);
    }
    assert_eq!(press_step(&mut ui, &t, "Enter"), Step::Run(key("c")));
}

#[test]
fn repeat_binding_leaves_prefix_table() {
    let mut t = tables();
    bind(
        &mut t,
        "prefix",
        "n",
        &["next-window"],
        Some("window"),
        true,
    );
    let mut keys = KeyState::new();
    let settings = KeySettings::default();
    let modes = InputModes::default();
    let routed = press(&mut keys, &t, "prefix", &key("n"), &settings, &modes);
    assert!(matches!(routed, Routed::Bound(..)));
    assert_eq!(keys.table(), "prefix");
    let routed = press(&mut keys, &t, "prefix", &key("c"), &settings, &modes);
    assert!(matches!(routed, Routed::Bound(..)));
    assert_eq!(keys.table(), "root");
}

#[test]
fn switch_client_t_opens_submenu() {
    let mut t = tables();
    bind(
        &mut t,
        "prefix",
        "[",
        &["switch-client", "-T", "copy"],
        Some("copy"),
        false,
    );
    let mut ui = ListState::default();
    assert_eq!(press_step(&mut ui, &t, "["), Step::Submenu("copy".into()));
}

#[test]
fn no_group_goes_to_other() {
    let t = tables();
    let r = rows(&t);
    let groups: Vec<&str> = r.iter().map(|r| r.group.as_str()).collect();
    assert_eq!(groups, ["window", "pane", "other"]);
    assert_eq!(r[2].key, key("z"));
}

#[test]
fn mouse_and_any_hidden() {
    let mut t = tables();
    bind(&mut t, "prefix", "Any", &["display", "x"], None, false);
    bind(
        &mut t,
        "prefix",
        "MouseDown1Pane",
        &["select-pane"],
        None,
        false,
    );
    let r = rows(&t);
    assert_eq!(r.len(), 3);
    assert!(
        r.iter()
            .all(|r| !matches!(r.key.code, KeyCode::Mouse(_) | KeyCode::Any))
    );
}

#[test]
fn labels_are_formats() {
    let state = crate::server::test_state();
    let ctx = crate::server::test_ctx();
    let mut t = tables();
    let mut b = Binding::new(
        vec![vec!["agent".into(), "next".into()]],
        false,
        BindingSource::Default,
    );
    b.group = Some("agents".into());
    b.desc = Some("Agents (#{e|+:1,2})".into());
    t.insert("prefix", key("a"), b);
    let r = build_rows(&t, "prefix", &|s| prompt::expand_command(&state, &ctx, s));
    let agents = r.iter().find(|r| r.key == key("a")).expect("row");
    assert_eq!(agents.label, "Agents (3)");
}
/// A server with a client attached to session `a`, running one agent.
fn client_state() -> (State, ClientId) {
    use crate::protocol::ClientCtx;
    use crate::server::clients::AttachRequest;
    use crate::server::{dispatch, test_ctx, test_state};
    let mut state = test_state();
    let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    dispatch::run(
        &mut state,
        &test_ctx(),
        &argv(&["new-session", "-d", "-s", "a", "sleep 600"]),
    )
    .expect("session");
    let accepted = clients::attach(
        &mut state,
        AttachRequest {
            ctx: ClientCtx {
                client: None,
                ..test_ctx()
            },
            size: (100, 40),
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
    (state, accepted.id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn label_expands_with_clients_pane() {
    use crate::server::agents::{AgentState, Report, ReportState};
    let (mut state, client) = client_state();
    let pane = *state.model.panes.keys().next().expect("pane");
    let report = Report {
        state: ReportState::Set(AgentState::Blocked),
        kind: Some("claude".into()),
        msg: None,
        blocked: None,
        session_id: None,
        ts: 1,
    };
    state.agents.apply(pane, report, true, Instant::now());
    let mut b = Binding::new(
        vec![vec!["choose-agent".into()]],
        false,
        BindingSource::Default,
    );
    b.group = Some("agents".into());
    b.desc = Some("Agents (#{agents_attention})".into());
    state.registry.key_tables.insert("prefix", key("U"), b);
    let rows = rows_for(&state, client, "prefix");
    let row = rows.iter().find(|r| r.key == key("U")).expect("row");
    assert_eq!(row.label, "Agents (1)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prefix_key_is_ignored_in_menu() {
    let (mut state, client) = client_state();
    let mut b = Binding::new(
        vec![vec!["display-message".into(), "x".into()]],
        false,
        BindingSource::Default,
    );
    b.group = Some("other".into());
    state.registry.key_tables.insert("custom", key("C-b"), b);
    open(&mut state, client, "custom").expect("open");
    let routed = KeyMenuHook.on_input(&mut state, client, &Input::Key(key("C-b")), Instant::now());
    assert!(matches!(routed, Some(Routed::Consumed)));
    assert!(state.keymenus.is_open(client), "menu stays");
    assert_eq!(state.clients[&client].keys.table(), "root");
}

#[test]
fn navigation_keys_belong_to_the_menu_not_the_bindings() {
    let mut t = tables();
    for name in ["Up", "Down", "PageUp", "PageDown", "Home", "End"] {
        bind(&mut t, "prefix", name, &["select-pane"], Some("pane"), true);
    }
    let mut ui = ListState::default();
    for name in ["Down", "Down", "Up", "PageDown", "PageUp", "End", "Home"] {
        assert_eq!(press_step(&mut ui, &t, name), Step::Redraw, "{name}");
    }
    assert_eq!(ui.selected, 0);
    // With a modifier the key is the binding's again.
    bind(&mut t, "prefix", "C-Up", &["resize-pane"], None, true);
    assert_eq!(press_step(&mut ui, &t, "C-Up"), Step::Run(key("C-Up")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn down_moves_the_selection_and_a_page_key_a_screenful() {
    let (mut state, client) = client_state();
    let mut b = Binding::new(
        vec![vec!["select-pane".into(), "-D".into()]],
        true,
        BindingSource::Default,
    );
    b.group = Some("pane".into());
    for name in ["Down", "NPage", "PPage"] {
        state
            .registry
            .key_tables
            .insert("prefix", key(name), b.clone());
    }
    open(&mut state, client, "prefix").expect("open");
    let size = state.clients[&client].size;
    let press_key = |state: &mut State, name: &str| {
        KeyMenuHook.on_input(state, client, &Input::Key(key(name)), Instant::now())
    };
    press_key(&mut state, "Down");
    assert!(
        state.keymenus.is_open(client),
        "Down does not run the binding"
    );
    assert_eq!(state.keymenus.open[&client].ui.selected, 1);
    press_key(&mut state, "NPage");
    let page = list_height(size, false);
    assert!(page > 10, "a screenful at 100x40 is more than the old 10");
    assert_eq!(state.keymenus.open[&client].ui.selected, 1 + page);
    press_key(&mut state, "PPage");
    assert_eq!(state.keymenus.open[&client].ui.selected, 1);
    assert_eq!(state.clients[&client].keys.table(), "root");
}

/// The screen's box rows as text, with the box's border removed.
fn drawn(state: &State, client: ClientId, size: (u16, u16)) -> Vec<String> {
    let mut screen = Screen::new(size.0, size.1);
    let cx = OverlayCtx {
        client,
        viewport: (0, 0),
        status_row: None,
        window_top: 0,
        panes: Vec::new(),
    };
    KeyMenuOverlay.draw(state, client, &cx, &mut screen);
    screen
        .cells
        .chunks(usize::from(size.0))
        .map(|r| r.iter().map(|c| c.c).collect::<String>())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn small_client_cuts_descriptions_with_an_ellipsis_only() {
    let (mut state, client) = client_state();
    state.clients.get_mut(&client).expect("client").size = (80, 24);
    open(&mut state, client, "prefix").expect("open");
    let lines = drawn(&state, client, (80, 24));
    let width = draw::area((80, 24)).cols;
    assert!(width >= 78, "most of an 80 column client, got {width}");
    assert!(
        lines.iter().any(|l| l.contains("╭─ Keys · prefix")),
        "{lines:#?}"
    );
    let seen = lines.iter().filter(|l| l.contains("  ")).count();
    assert!(seen > 8, "rows drawn: {seen}");
    for line in &lines {
        for (at, _) in line.match_indices('…') {
            let next = line[at + '…'.len_utf8()..].chars().next();
            assert!(matches!(next, Some(' ' | '│')), "cut mid-cell: {line:?}");
        }
    }
    assert!(
        lines
            .iter()
            .any(|l| l.contains("type a key to run · C-f search")),
        "the hint is in the box, and names C-f as prefix / is bound: {lines:#?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detach_removes_menu() {
    let (mut state, client) = client_state();
    open(&mut state, client, "prefix").expect("open");
    clients::detach(&mut state, client);
    assert!(!state.keymenus.is_open(client));
}

#[test]
fn slash_searches_unless_bound() {
    let mut t = tables();
    let mut ui = ListState::default();
    assert_eq!(press_step(&mut ui, &t, "/"), Step::Redraw);
    assert!(ui.filtering);
    bind(&mut t, "prefix", "/", &["list-keys"], None, false);
    let mut ui = ListState::default();
    assert_eq!(press_step(&mut ui, &t, "/"), Step::Run(key("/")));
    assert_eq!(press_step(&mut ui, &t, "C-f"), Step::Redraw);
    assert!(ui.filtering, "C-f still searches");
}

#[test]
fn enter_in_search_runs_the_selected_result() {
    let t = tables();
    let mut ui = ListState::default();
    press_step(&mut ui, &t, "/");
    for c in ["k", "i", "l"] {
        assert_eq!(press_step(&mut ui, &t, c), Step::Redraw);
    }
    assert_eq!(press_step(&mut ui, &t, "Enter"), Step::Run(key("x")));
}

#[test]
fn escape_leaves_search_then_closes() {
    let t = tables();
    let mut ui = ListState::default();
    press_step(&mut ui, &t, "C-f");
    press_step(&mut ui, &t, "w");
    assert_eq!(press_step(&mut ui, &t, "Escape"), Step::Redraw);
    assert!(!ui.filtering && ui.query.is_empty());
    assert_eq!(
        press_step(&mut ui, &t, "c"),
        Step::Run(key("c")),
        "keys run again"
    );
    assert_eq!(press_step(&mut ui, &t, "Escape"), Step::Close);
}

#[test]
fn backspace_on_an_empty_query_leaves_search() {
    let t = tables();
    let mut ui = ListState::default();
    press_step(&mut ui, &t, "/");
    press_step(&mut ui, &t, "w");
    assert_eq!(press_step(&mut ui, &t, "BSpace"), Step::Redraw);
    assert!(ui.filtering && ui.query.is_empty());
    assert_eq!(press_step(&mut ui, &t, "BSpace"), Step::Redraw);
    assert!(!ui.filtering);
}

#[test]
fn groups_follow_the_defaults_order_then_first_seen() {
    let mut t = tables();
    bind(
        &mut t,
        "prefix",
        "!",
        &["break-pane"],
        Some("zz-mine"),
        false,
    );
    bind(
        &mut t,
        "prefix",
        "s",
        &["choose-tree"],
        Some("session"),
        false,
    );
    let r = rows(&t);
    let mut groups: Vec<&str> = r.iter().map(|r| r.group.as_str()).collect();
    groups.dedup();
    assert_eq!(groups, ["window", "pane", "session", "zz-mine", "other"]);
}
