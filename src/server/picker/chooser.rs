//! The chooser `grofe.choose` opens: the picker's list, filter and keys over
//! items a script gives, with an optional preview text per item. Rows are
//! the script's labels; the pick goes back to its callback.

use std::collections::BTreeMap;

use crate::lua::api::{ChooseJob, Payload};
use crate::protocol::ClientCtx;
use crate::server::input::{Input, Routed};
use crate::server::listdraw::{Rows, centred, draw_list, put_text};
use crate::server::listview::{ListAction, ListState, filter};
use crate::server::render::Screen;
use crate::server::status::Style;
use crate::server::{State, clients, lua_async, lua_host};
use crate::types::{ClientId, Rect};

/// Size of the box, percent of the client.
const WIDTH_PCT: u16 = 70;
const HEIGHT_PCT: u16 = 60;

/// One open chooser: the callback it answers, the context that callback
/// runs in, and what it lists.
pub(super) struct Choice {
    generation: u64,
    id: u32,
    press: ClientCtx,
    job: ChooseJob,
    list: ListState,
}

/// Open choosers, per client.
#[derive(Default)]
pub(super) struct Choosers {
    open: BTreeMap<ClientId, Choice>,
}

/// Opens a chooser on `client` for callback `id` of runtime `generation`.
pub(crate) fn open(
    state: &mut State,
    client: ClientId,
    press: ClientCtx,
    (generation, id): (u64, u32),
    job: ChooseJob,
) {
    let choice = Choice {
        generation,
        id,
        press,
        job,
        list: ListState::default(),
    };
    // A chooser opened over another answers that one with nothing.
    if let Some(old) = state.pickers.choosers.open.insert(client, choice) {
        answer(state, old, None);
    }
    clients::mark_dirty(state, client);
}

/// A detached client takes its chooser with it; its callback gets nothing.
pub(super) fn client_gone(state: &mut State, client: ClientId) {
    if let Some(c) = state.pickers.choosers.open.remove(&client) {
        answer(state, c, None);
    }
}

/// A reload ends the runtime the open choosers' callbacks live in, so they
/// close unanswered and their callbacks go with it.
pub(crate) fn clear(state: &mut State) {
    let open = std::mem::take(&mut state.pickers.choosers.open);
    for client in open.into_keys() {
        clients::mark_dirty(state, client);
    }
}

pub(super) fn is_open(state: &State, client: ClientId) -> bool {
    state.pickers.choosers.open.contains_key(&client)
}

/// Queues the callback's call with row `pick` (0-based) or nothing, for
/// whichever answer ends the chooser (pick, cancel, replaced, detached). It
/// never runs inline, as the runtime may be out of `state` right now; if the
/// queue is full the callback is freed unanswered.
fn answer(state: &State, c: Choice, pick: Option<usize>) {
    let Choice {
        generation,
        id,
        press,
        ..
    } = c;
    let pick = pick.map(|i| i + 1);
    let job = lua_async::guarded(generation, move |state| {
        lua_host::call_once_for(state, &press, id, Payload::Pick(pick));
    });
    if state.jobs.try_send(job).is_err() {
        lua_host::release_once(state, generation, id);
    }
}

fn visible(c: &Choice) -> Vec<usize> {
    filter(&c.job.labels, &c.list.query)
}

/// The list box and, when there are previews, the box under it.
fn layout(size: (u16, u16), previews: bool) -> (Rect, Option<Rect>) {
    let area = centred(size, WIDTH_PCT, HEIGHT_PCT);
    if !previews || area.rows < 8 {
        return (area, None);
    }
    let rows = area.rows * 2 / 5;
    let list = Rect { rows, ..area };
    let preview = Rect {
        y: area.y + rows,
        rows: area.rows - rows,
        ..area
    };
    (list, Some(preview))
}

/// Rows of the list box that hold items: the box less its border and the
/// query line.
fn list_height(c: &Choice, size: (u16, u16)) -> usize {
    let (list, _) = layout(size, !c.job.previews.is_empty());
    usize::from(list.rows).saturating_sub(3)
}

pub(super) fn on_input(state: &mut State, client: ClientId, input: &Input) -> Option<Routed> {
    let Some(size) = state.clients.get(&client).map(|c| c.size) else {
        client_gone(state, client);
        return None;
    };
    let key = match input {
        Input::Focus(_) => return None,
        Input::Key(key) => key,
        Input::Mouse(_) | Input::Paste { .. } => return Some(Routed::Consumed),
    };
    let c = state.pickers.choosers.open.get_mut(&client)?;
    let height = list_height(c, size);
    let count = visible(c).len();
    c.list.clamp(count, height);
    let action = c.list.on_key(key, count, true);
    let rows = visible(c);
    c.list.clamp(rows.len(), height);
    let picked = rows.get(c.list.selected).copied();
    clients::mark_dirty(state, client);
    match action {
        ListAction::Close => close(state, client, None),
        ListAction::Enter => {
            if let Some(i) = picked {
                close(state, client, Some(i));
            }
        }
        _ => {}
    }
    Some(Routed::Consumed)
}

fn close(state: &mut State, client: ClientId, pick: Option<usize>) {
    if let Some(c) = state.pickers.choosers.open.remove(&client) {
        answer(state, c, pick);
    }
    clients::mark_dirty(state, client);
}

/// Draws client's chooser, if one is open; whether it did.
pub(super) fn draw(state: &State, client: ClientId, screen: &mut Screen) -> bool {
    let Some(c) = state.pickers.choosers.open.get(&client) else {
        return false;
    };
    let (list, preview) = layout((screen.cols, screen.rows), !c.job.previews.is_empty());
    let rows = Rows {
        cols: c.job.labels.iter().map(|l| vec![l.clone()]).collect(),
    };
    let shown = visible(c);
    let title = c.job.title.as_deref().unwrap_or("choose");
    draw_list(screen, list, title, &rows, &shown, &c.list, true);
    screen.cursor = None;
    if let (Some(area), Some(text)) = (
        preview.filter(|a| a.cols >= 3),
        shown
            .get(c.list.selected)
            .and_then(|&i| c.job.previews.get(i)),
    ) {
        draw_list(
            screen,
            area,
            "",
            &Rows::default(),
            &[],
            &ListState::default(),
            false,
        );
        let (x, end) = (usize::from(area.x) + 1, usize::from(area.x + area.cols) - 1);
        let last = usize::from(area.y + area.rows) - 1;
        for (n, line) in text.lines().enumerate() {
            let y = usize::from(area.y) + 1 + n;
            if y >= last {
                break;
            }
            put_text(screen, x, y, end, line, &Style::default());
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{test_ctx, test_state};

    #[test]
    fn a_preview_is_skipped_in_a_box_under_three_columns() {
        let mut state = test_state();
        let job = ChooseJob {
            labels: vec!["a".into()],
            previews: vec!["PREVIEW".into()],
            title: None,
        };
        state.pickers.choosers.open.insert(
            ClientId(1),
            Choice {
                generation: 0,
                id: 1,
                press: test_ctx(),
                job,
                list: ListState::default(),
            },
        );
        for cols in [0, 1, 2, 3, 4, 5] {
            let mut screen = Screen::new(cols, 20);
            assert!(draw(&state, ClientId(1), &mut screen));
            let (_, preview) = layout((cols, 20), true);
            let wide = preview.is_some_and(|a| a.cols >= 3);
            assert_eq!(wide, cols >= 5, "{cols} columns");
            let text: String = screen.cells.iter().map(|c| c.c).collect();
            assert_eq!(text.contains('P'), wide, "{cols} columns");
        }
    }
}
