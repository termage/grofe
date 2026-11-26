//! The agent grid (agents spec §5.4).

use std::collections::BTreeMap;
use std::time::Instant;

mod draw;

use super::State;
use super::agents::{Agent, Rank, rank};
use super::borders::PaneLines;
use super::clients;
use super::dispatch::agent;
use super::dispatch::options::{FormatCtx, ServerVars};
use super::frameopts;
use super::input::{Input, InputHook, KeyCode, Routed};
use super::listdraw::put_text;
use super::listview::{ListAction, ListState, filter};
use super::pane::{CellFlags, SnapCell};
use super::render::{Overlay, OverlayCtx, Screen};
use super::status::Style;
use crate::cmd::expand;
use crate::types::{ClientId, PaneId};
use draw::{Layout, Look, Tile, blank};

/// Open grids, per client. Lives on `State.grids`. `ListState.selected` is
/// the tile, `top` the first tile row shown.
#[derive(Default)]
pub struct Grids {
    open: BTreeMap<ClientId, ListState>,
    /// When a shown time next changes (see [`deadline`]).
    at: Option<Instant>,
}

impl Grids {
    pub fn is_open(&self, id: ClientId) -> bool {
        self.open.contains_key(&id)
    }
}

pub fn open(state: &mut State, client: ClientId) -> Result<(), String> {
    if !state.clients.contains_key(&client) {
        return Err("no current client".to_string());
    }
    state.grids.open.insert(client, ListState::default());
    rearm(state, Instant::now());
    clients::mark_dirty(state, client);
    Ok(())
}

/// When the open grids next need a redraw for their times.
pub fn deadline(state: &State) -> Option<Instant> {
    state.grids.at
}

/// Redraws the open grids when a shown time has changed.
pub fn tick(state: &mut State, now: Instant) {
    if !state.grids.at.is_some_and(|at| at <= now) {
        return;
    }
    let ids: Vec<ClientId> = state.grids.open.keys().copied().collect();
    for id in ids {
        clients::mark_dirty(state, id);
    }
    rearm(state, now);
}

fn rearm(state: &mut State, now: Instant) {
    state.grids.at = if state.grids.open.is_empty() {
        None
    } else {
        super::picker::next_time_change(state, now)
    };
}

/// A client that detached takes its grid with it.
pub fn client_gone(state: &mut State, id: ClientId) {
    state.grids.open.remove(&id);
    rearm(state, Instant::now());
}

fn layout(state: &State, size: (u16, u16)) -> Layout {
    draw::layout(
        size,
        state.agents_grid.tile_width,
        state.agents_grid.preview_lines,
    )
}

fn target_label(state: &State, pane: PaneId) -> String {
    if let Some(label) = state.agents.get(pane).and_then(|a| a.label.clone()) {
        return label;
    }
    let vars = FormatCtx::new(&state.model, pane);
    expand(
        "#{session_name}:#{window_index}.#{pane_index}",
        &ServerVars::fresh(state, pane, &vars),
    )
}

fn age(since: Instant) -> String {
    let secs = since.elapsed().as_secs();
    match secs {
        0..60 => format!("0:{secs:02}"),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

fn glyph(a: &Agent) -> &'static str {
    match rank(a) {
        Rank::Blocked => "⏸",
        Rank::Error => "✗",
        Rank::Unseen | Rank::Done => "✓",
        Rank::Working => "●",
        Rank::Idle => "○",
    }
}

fn title(state: &State, pane: PaneId, a: &Agent) -> String {
    let head = format!(
        "{} {} {} {}",
        glyph(a),
        target_label(state, pane),
        a.kind.clone().unwrap_or_default(),
        age(a.since)
    );
    match message(a) {
        "" => head,
        m => format!("{head} {m}"),
    }
}

/// What the agent last said: its blocked reason, else its message.
fn message(a: &Agent) -> &str {
    a.blocked.as_deref().or(a.msg.as_deref()).unwrap_or("")
}

/// The agent panes in display order that match `query`, with their titles.
fn view(state: &State, query: &str) -> Vec<(PaneId, String)> {
    let all: Vec<(PaneId, String)> = state
        .agents
        .by_urgency()
        .into_iter()
        .filter(|p| state.model.panes.contains_key(p) || state.agents.is_headless(*p))
        .filter_map(|p| state.agents.get(p).map(|a| (p, title(state, p, a))))
        .collect();
    let texts: Vec<String> = all
        .iter()
        .map(|(p, t)| {
            let state_word = state.agents.get(*p).map_or("", |a| rank(a).as_str());
            format!("{t} {state_word}")
        })
        .collect();
    filter(&texts, query)
        .into_iter()
        .map(|i| all[i].clone())
        .collect()
}

type Tiles = (Vec<(PaneId, String)>, Layout, ListState);

fn tiles_of(state: &State, client: ClientId) -> Option<Tiles> {
    let st = state.grids.open.get(&client)?;
    let size = state.clients.get(&client)?.size;
    Some((view(state, &st.query), layout(state, size), st.clone()))
}

/// The panes whose tiles are on screen for `client`; empty if no grid is open.
pub fn visible_panes(state: &State, client: ClientId) -> Vec<PaneId> {
    let Some((tiles, lay, st)) = tiles_of(state, client) else {
        return Vec::new();
    };
    tiles
        .iter()
        .skip(st.top * lay.cols)
        .take(lay.rows * lay.cols)
        .map(|(p, _)| *p)
        .collect()
}

/// Keeps the selection inside the tiles and its row on screen.
fn clamp(st: &mut ListState, count: usize, lay: &Layout) {
    st.selected = st.selected.min(count.saturating_sub(1));
    let row = st.selected / lay.cols;
    if row < st.top {
        st.top = row;
    } else if row >= st.top + lay.rows {
        st.top = row + 1 - lay.rows;
    }
    let total_rows = count.div_ceil(lay.cols);
    st.top = st.top.min(total_rows.saturating_sub(lay.rows));
}

pub struct GridHook;

impl InputHook for GridHook {
    fn on_input(
        &self,
        state: &mut State,
        client: ClientId,
        input: &Input,
        _now: Instant,
    ) -> Option<Routed> {
        if !state.grids.is_open(client) {
            return None;
        }
        let Some(size) = state.clients.get(&client).map(|c| c.size) else {
            client_gone(state, client);
            return None;
        };
        let key = match input {
            Input::Focus(_) => return None,
            Input::Key(key) => key,
            Input::Mouse(_) | Input::Paste { .. } => return Some(Routed::Consumed),
        };
        let lay = layout(state, size);
        let mut st = state.grids.open.get(&client).cloned()?;
        let count = view(state, &st.query).len();
        let plain = !(key.mods.ctrl || key.mods.alt || key.mods.super_);
        let step = match key.code {
            KeyCode::Left if plain => Some((-1, 1)),
            KeyCode::Right if plain => Some((1, 1)),
            KeyCode::Up if plain => Some((-1, lay.cols)),
            KeyCode::Down if plain => Some((1, lay.cols)),
            _ => None,
        };
        let action = if let Some((dir, by)) = step {
            st.selected = if dir < 0 {
                st.selected.saturating_sub(by)
            } else {
                (st.selected + by).min(count.saturating_sub(1))
            };
            ListAction::Moved
        } else {
            st.on_key(key, count, true)
        };
        let count = view(state, &st.query).len();
        clamp(&mut st, count, &lay);
        state.grids.open.insert(client, st);
        clients::mark_dirty(state, client);
        Some(match action {
            ListAction::Close => close(state, client),
            ListAction::Enter => match selected_pane(state, client) {
                Some(pane) => enter(state, client, pane),
                None => Routed::Consumed,
            },
            _ => Routed::Consumed,
        })
    }
}

/// The pane under the selection, if the grid is open and shows any.
pub fn selected_pane(state: &State, client: ClientId) -> Option<PaneId> {
    let (tiles, _, st) = tiles_of(state, client)?;
    tiles.get(st.selected).map(|(p, _)| *p)
}

fn close(state: &mut State, client: ClientId) -> Routed {
    state.grids.open.remove(&client);
    rearm(state, Instant::now());
    clients::mark_dirty(state, client);
    Routed::Consumed
}

fn enter(state: &mut State, client: ClientId, pane: PaneId) -> Routed {
    let ctx = state.clients.get(&client).map(|c| c.ctx.clone());
    close(state, client);
    if let Some(ctx) = ctx
        && let Err(e) = agent::jump(state, &ctx, pane)
    {
        clients::show_message(state, client, &e);
    }
    Routed::Consumed
}

/// The frame looks of the client's window, as the sidebar takes them:
/// its lines, `pane-border-style` and `pane-active-border-style`.
fn look(state: &State, client: ClientId) -> Look {
    frameopts::with_frame_options(state, client, |f| Look {
        lines: f.lines,
        border: f.style("pane-border-style"),
        active: f.style("pane-active-border-style"),
    })
    .unwrap_or(Look {
        lines: PaneLines::parse(""),
        border: Style::default(),
        active: Style::default(),
    })
}

/// The pane's number in its window, as `#{pane_index}` shows it (what
/// `pane-border-lines number` draws).
fn pane_index(state: &State, pane: PaneId) -> usize {
    let m = &state.model;
    let Some(w) = m.panes.get(&pane).and_then(|p| m.windows.get(&p.window)) else {
        return 0;
    };
    let pos = w.pane_ids().iter().position(|id| *id == pane).unwrap_or(0);
    pos + m.pane_base_index(w.id) as usize
}

/// The last rows of `pane`'s screen, `h` at most.
fn body(state: &State, pane: PaneId, h: usize) -> Vec<Vec<SnapCell>> {
    let Some(p) = state.model.panes.get(&pane) else {
        return Vec::new();
    };
    let h = u16::try_from(h).unwrap_or(u16::MAX);
    p.handle
        .snapshot_rows(p.rows.saturating_sub(h)..p.rows)
        .rows
}

pub struct GridOverlay;

impl Overlay for GridOverlay {
    fn draw(&self, state: &State, client: ClientId, _cx: &OverlayCtx, screen: &mut Screen) {
        let Some(st) = state.grids.open.get(&client) else {
            return;
        };
        let lay = layout(state, (screen.cols, screen.rows));
        let found = view(state, &st.query);
        screen.cells.fill(blank());
        screen.cursor = None;
        let cols = usize::from(screen.cols);
        let bold = Style {
            attrs: CellFlags::BOLD,
            ..Style::default()
        };
        put_text(screen, 0, 0, cols, &format!("agents> {}", st.query), &bold);
        if found.is_empty() {
            put_text(screen, 0, 1, cols, "no agents", &Style::default());
            return;
        }
        let first = st.top * lay.cols;
        let lines = usize::from(state.agents_grid.preview_lines);
        let tiles: Vec<Tile> = found
            .into_iter()
            .enumerate()
            .map(|(i, (pane, label))| {
                let shown = (first..first + lay.rows * lay.cols).contains(&i);
                Tile {
                    label,
                    index: if shown { pane_index(state, pane) } else { 0 },
                    body: if shown {
                        body(state, pane, lines)
                    } else {
                        Vec::new()
                    },
                }
            })
            .collect();
        draw::draw_tiles(
            screen,
            &lay,
            &tiles,
            first,
            st.selected,
            &look(state, client),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ClientCtx;
    use crate::server::agents::{AgentState, Report, ReportState};
    use crate::server::clients::AttachRequest;
    use crate::server::input::parse_key;
    use crate::server::status;
    use crate::server::{dispatch, test_ctx, test_state};

    fn report(state: AgentState) -> Report {
        Report {
            state: ReportState::Set(state),
            kind: Some("claude".into()),
            msg: None,
            blocked: None,
            session_id: None,
            ts: 1,
        }
    }

    fn run(state: &mut State, args: &[&str]) {
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        dispatch::run(state, &test_ctx(), &argv).expect("command");
    }

    /// `n` agent panes in session `a`, one client of `size`. The first
    /// pane is blocked, the rest idle.
    fn setup(
        n: usize,
        size: (u16, u16),
    ) -> (
        State,
        ClientId,
        Vec<PaneId>,
        tokio::sync::mpsc::Receiver<Vec<u8>>,
    ) {
        let mut state = test_state();
        run(&mut state, &["new-session", "-d", "-s", "a", "sleep 600"]);
        for _ in 1..n {
            run(&mut state, &["new-window", "-t", "a", "sleep 600"]);
        }
        let panes: Vec<PaneId> = state.model.panes.keys().copied().collect();
        let now = Instant::now();
        for (i, p) in panes.iter().enumerate() {
            let s = if i == 0 {
                AgentState::Blocked
            } else {
                AgentState::Idle
            };
            state.agents.apply(*p, report(s), true, now);
        }
        let accepted = clients::attach(
            &mut state,
            AttachRequest {
                ctx: ClientCtx {
                    client: None,
                    ..test_ctx()
                },
                size,
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
        (state, accepted.id, panes, accepted.output)
    }

    fn press(state: &mut State, client: ClientId, name: &str) -> Option<Routed> {
        let key = parse_key(name).expect("key");
        GridHook.on_input(state, client, &Input::Key(key), Instant::now())
    }

    fn screen_text(state: &State, client: ClientId, size: (u16, u16)) -> String {
        let screen = draw_screen(state, client, size);
        screen
            .cells
            .chunks(usize::from(size.0))
            .map(|r| r.iter().map(|c| c.c).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn draw_screen(state: &State, client: ClientId, size: (u16, u16)) -> Screen {
        let mut screen = Screen::new(size.0, size.1);
        let cx = OverlayCtx {
            client,
            viewport: (0, 0),
            status_row: None,
            window_top: 0,
            panes: Vec::new(),
        };
        GridOverlay.draw(state, client, &cx, &mut screen);
        screen
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn arrows_move_enter_jumps() {
        let (mut state, client, panes, _out) = setup(5, (130, 40));
        open(&mut state, client).expect("open");
        assert_eq!(selected_pane(&state, client), Some(panes[0]));
        press(&mut state, client, "Right");
        let second = selected_pane(&state, client).expect("tile");
        assert_ne!(second, panes[0]);
        press(&mut state, client, "Down");
        let fourth = selected_pane(&state, client).expect("tile");
        assert_ne!(fourth, second);
        press(&mut state, client, "Left");
        press(&mut state, client, "Up");
        assert_eq!(selected_pane(&state, client), Some(panes[0]));
        press(&mut state, client, "Right");
        press(&mut state, client, "Enter");
        assert!(!state.grids.is_open(client));
        let window = state.model.panes[&second].window;
        assert_eq!(state.model.windows[&window].active, second);
        let session = state.clients[&client].session;
        assert_eq!(state.model.sessions[&session].active, window);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn columns_and_scroll() {
        let (mut state, client, _panes, _out) = setup(5, (130, 45));
        let lay = layout(&state, (130, 45));
        assert_eq!(lay.cols, 2);
        // 45 rows hold three tile rows of 14: 5 agents are 3 rows, all shown.
        open(&mut state, client).expect("open");
        assert_eq!(visible_panes(&state, client).len(), 5);
        // A short client scrolls by tile row and keeps the selection shown.
        state.clients.get_mut(&client).expect("client").size = (130, 15);
        for _ in 0..4 {
            press(&mut state, client, "Down");
        }
        let shown = visible_panes(&state, client);
        assert_eq!(shown.len(), 1, "the last tile row holds one tile");
        assert!(shown.contains(&selected_pane(&state, client).expect("tile")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn frames_follow_the_border_options() {
        let (mut state, client, _panes, _out) = setup(2, (130, 40));
        run(&mut state, &["set", "-g", "pane-border-style", "fg=red"]);
        run(
            &mut state,
            &["set", "-g", "pane-active-border-style", "fg=green"],
        );
        run(&mut state, &["set", "-g", "pane-border-lines", "double"]);
        open(&mut state, client).expect("open");
        let lay = layout(&state, (130, 40));
        let s = draw_screen(&state, client, (130, 40));
        let cell = |x: usize, y: usize| &s.cells[y * 130 + x];
        let (x0, y0, w0, h0) = lay.rect(0);
        let (x1, y1, ..) = lay.rect(1);
        let green = status::parse_style("fg=green").fg;
        let red = status::parse_style("fg=red").fg;
        assert_eq!(cell(x0, y0).c, '╔');
        assert_eq!(cell(x0, y0).fg, green, "the selected tile");
        assert_eq!(cell(x0 + w0 - 1, y0 + h0 - 1).c, '╝');
        assert_eq!(cell(x0 + w0 - 1, y0 + h0 - 1).fg, green);
        assert_eq!(cell(x1, y1).c, '╔');
        assert_eq!(cell(x1, y1).fg, red, "an unselected tile");
        press(&mut state, client, "Right");
        let s = draw_screen(&state, client, (130, 40));
        assert_eq!(s.cells[y1 * 130 + x1].fg, green, "the selection moved");
        assert_eq!(s.cells[y0 * 130 + x0].fg, red);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn number_lines_show_the_pane_index() {
        // Two agent panes in one window, numbered from pane-base-index.
        let (mut state, client, panes, _out) = setup(1, (130, 40));
        run(&mut state, &["set", "-g", "pane-base-index", "3"]);
        run(&mut state, &["split-window", "-t", "a", "sleep 600"]);
        let second = *state
            .model
            .panes
            .keys()
            .find(|p| **p != panes[0])
            .expect("second pane");
        state
            .agents
            .apply(second, report(AgentState::Idle), true, Instant::now());
        run(&mut state, &["set", "-g", "pane-border-lines", "number"]);
        open(&mut state, client).expect("open");
        let lay = layout(&state, (130, 40));
        let s = draw_screen(&state, client, (130, 40));
        let order: Vec<PaneId> = view(&state, "").into_iter().map(|(p, _)| p).collect();
        assert_eq!(order, vec![panes[0], second]);
        for (slot, want) in [(0, '3'), (1, '4')] {
            let (x, y, w, h) = lay.rect(slot);
            assert_eq!(
                s.cells[(y + h - 1) * 130 + x + w - 1].c,
                want,
                "tile {slot}"
            );
        }
    }

    /// Spec §2.7 on the grid alone: format markup in an agent's message
    /// and in a session name is drawn as text, opens no style and runs no
    /// job.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn agent_text_is_inert_in_tiles() {
        // A short marker dir: a long temp path would push the probes past
        // the title's width.
        let dir = std::path::PathBuf::from(format!(
            "/tmp/gg{}{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("marker dir");
        let (x, y) = (dir.join("X"), dir.join("Y"));
        let name = format!("s#[fg=red]#(touch {})", y.display());
        let msg = format!("D#[range=user|x]E##[fg=red]F#(touch {})", x.display());
        let mut state = test_state();
        run(&mut state, &["new-session", "-d", "-s", &name, "sleep 600"]);
        let pane = *state.model.panes.keys().next().expect("pane");
        let mut r = report(AgentState::Blocked);
        r.msg = Some(msg.clone());
        state.agents.apply(pane, r, true, Instant::now());
        state.agents_grid.tile_width = 200;
        let accepted = clients::attach(
            &mut state,
            AttachRequest {
                ctx: ClientCtx {
                    client: None,
                    ..test_ctx()
                },
                size: (160, 40),
                term: "xterm".into(),
                caps: Default::default(),
                pid: None,
                target: None,
                detach_others: false,
                nest: false,
                tty: None,
                created: false,
                extras: Default::default(),
            },
        )
        .expect("attach");
        open(&mut state, accepted.id).expect("open");
        let s = draw_screen(&state, accepted.id, (160, 40));
        let top: String = s.cells[160..320].iter().map(|c| c.c).collect();
        assert!(
            top.contains(&format!("{name}:")),
            "session name drawn: {top:?}"
        );
        // The message as stored (agents sanitise it on report), drawn as is.
        let stored = state.agents.get(pane).and_then(|a| a.msg.clone());
        let stored = stored.expect("a message");
        assert!(stored.contains("#[range=user|x]"), "{stored:?}");
        assert!(top.contains(&stored), "message drawn: {top:?}");
        let red = status::parse_style("fg=red").fg;
        assert!(
            s.cells[160..320].iter().all(|c| c.fg != red),
            "no style opened"
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!x.exists() && !y.exists(), "a job ran agent text");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn typing_filters() {
        let (mut state, client, panes, _out) = setup(2, (130, 40));
        open(&mut state, client).expect("open");
        for c in ["b", "l", "o", "c", "k"] {
            press(&mut state, client, c);
        }
        assert_eq!(visible_panes(&state, client), vec![panes[0]]);
        assert!(screen_text(&state, client, (130, 40)).contains("agents> block"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn escape_closes() {
        let (mut state, client, _panes, _out) = setup(2, (130, 40));
        open(&mut state, client).expect("open");
        press(&mut state, client, "Escape");
        assert!(!state.grids.is_open(client));
        assert!(visible_panes(&state, client).is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empty_says_no_agents() {
        let mut state = test_state();
        run(&mut state, &["new-session", "-d", "-s", "a", "sleep 600"]);
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
        open(&mut state, accepted.id).expect("open");
        assert!(screen_text(&state, accepted.id, (100, 40)).contains("no agents"));
        press(&mut state, accepted.id, "Enter");
        assert!(state.grids.is_open(accepted.id));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tile_title_cut_by_width() {
        let (mut state, client, panes, _out) = setup(2, (130, 40));
        state.agents_grid.tile_width = 12;
        open(&mut state, client).expect("open");
        let lay = layout(&state, (130, 40));
        let (first, label) = view(&state, "").remove(0);
        let (_, _, w, _) = lay.rect(0);
        assert!(first == panes[0] && label.chars().count() > w);
        let text = screen_text(&state, client, (130, 40));
        let row: Vec<char> = text.lines().nth(1).expect("title row").chars().collect();
        let head: String = label.chars().take(3).collect();
        let top: String = row[..w].iter().collect();
        let corner = |c: char| c == '┌' || c == '╭';
        assert!(
            corner(row[0]) && top[3..].starts_with(&format!("─ {head}")),
            "{top}"
        );
        assert!(top.contains("… ─"), "{top}");
        assert!(
            corner(row[lay.rect(1).0]),
            "next tile starts at its own column"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn time_in_state_has_a_deadline_tick() {
        let (mut state, client, _panes, _out) = setup(2, (130, 40));
        assert_eq!(deadline(&state), None);
        open(&mut state, client).expect("open");
        let at = deadline(&state).expect("a deadline");
        clients::take_dirty(&mut state, client);
        tick(&mut state, at - std::time::Duration::from_millis(1));
        assert!(!clients::take_dirty(&mut state, client), "not yet");
        tick(&mut state, at);
        assert!(clients::take_dirty(&mut state, client), "the time changed");
        assert!(deadline(&state).is_some_and(|next| next > at));
        press(&mut state, client, "Escape");
        assert_eq!(deadline(&state), None);
        open(&mut state, client).expect("open");
        client_gone(&mut state, client);
        assert_eq!(deadline(&state), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn filter_matches_the_message() {
        let (mut state, _client, panes, _out) = setup(2, (130, 40));
        let mut r = report(AgentState::Idle);
        r.msg = Some("refactoring zebra".into());
        r.ts = 2;
        state.agents.apply(panes[1], r, true, Instant::now());
        let hit = view(&state, "zebra");
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].0, panes[1]);
        assert!(hit[0].1.ends_with("refactoring zebra"));
    }
}
