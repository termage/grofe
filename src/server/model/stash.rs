//! Scratchpad stashing: a hidden scratchpad leaves the pane map for
//! `Model::scratch`, and what leaves the model for good is noted for
//! `pane-closed`.

use super::{Floating, Model, Pane, Removed, Stashed};
use crate::event::Place;
use crate::types::{PaneId, SessionId, WindowId};

impl Model {
    /// Notes every pane as closed, hidden scratchpads and dead panes
    /// included: the server is going.
    pub(crate) fn note_all_closed(&mut self) {
        let all = self
            .panes
            .values()
            .chain(self.scratch.iter().map(|s| &s.pane));
        let places: Vec<_> = all.map(|p| (p.id, self.place_of(p))).collect();
        self.closed_panes.extend(places);
    }

    /// Notes that `pane` has left the model for good, for `pane-closed`
    /// (`Events::panes_closed`), and returns where it was. `session` is its
    /// session when its window is already gone. Every removal calls it, so a
    /// pane moved between windows never does.
    pub(crate) fn note_closed(&mut self, pane: &Pane, session: Option<SessionId>) -> Place {
        let mut place = self.place_of(pane);
        place.session = place.session.or(session);
        self.closed_panes.push((pane.id, place));
        place
    }

    /// Hides the scratchpad `pane`: it leaves its window (which goes with
    /// it if it was the last pane) and the pane map, and heads the history.
    /// Entries past `scratchpad-history` are dropped, their processes hung
    /// up; those returned were still live.
    pub(crate) fn stash_pane(&mut self, pane: PaneId) -> Result<Removed, String> {
        let window_id = self
            .panes
            .get(&pane)
            .map(|p| p.window)
            .ok_or_else(|| format!("no such pane: {pane}"))?;
        let place = self
            .windows
            .get(&window_id)
            .and_then(|w| w.floating(pane))
            .copied()
            .filter(|f| f.scratch)
            .ok_or("pane is not a scratchpad")?;
        let Some(window) = self.windows.get_mut(&window_id) else {
            return Err("pane is not a scratchpad".to_string());
        };
        let order = window.pane_ids();
        window.floating.retain(|f| f.pane != pane);
        window.lose_pane(&order, pane);
        let last = order.len() <= 1;
        let Some(taken) = self.panes.remove(&pane) else {
            return Ok(Vec::new());
        };
        // A dead pane (`remain-on-exit`) has no process to show again.
        let mut removed = if taken.dead.is_none() {
            self.push_stash(taken, place)
        } else {
            self.note_closed(&taken, None);
            Vec::new()
        };
        if last {
            removed.extend(self.remove_window(window_id));
        }
        Ok(removed)
    }

    /// Puts `pane` at the head of the history and drops what exceeds
    /// `scratchpad-history` (10 when unset or unreadable).
    pub(crate) fn push_stash(&mut self, pane: Pane, place: Floating) -> Removed {
        let limit = self
            .options
            .lookup("scratchpad-history", &[])
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(10);
        self.scratch.insert(0, Stashed { pane, place });
        let mut removed = Vec::new();
        while self.scratch.len() > limit {
            let Some(old) = self.scratch.pop() else {
                break;
            };
            let place = self.note_closed(&old.pane, None);
            if old.pane.dead.is_none() {
                removed.push((old.pane.id, old.pane.uid.clone(), place));
            }
        }
        removed
    }

    /// Shows the hidden scratchpad `pane` in `window`: on top at its last
    /// size, held inside the window, and active.
    pub(crate) fn unstash_pane(&mut self, pane: PaneId, window: WindowId) -> Result<(), String> {
        let Some(at) = self.scratch.iter().position(|s| s.pane.id == pane) else {
            return Err(format!("no such scratchpad: {pane}"));
        };
        let Some(w) = self.windows.get_mut(&window) else {
            return Err(format!("can't find window: {window}"));
        };
        let Stashed { mut pane, place } = self.scratch.remove(at);
        let mut place =
            crate::server::floating::placed(&place, (place.x, place.y), (w.cols, w.rows));
        place.z = w.floating.iter().map(|f| f.z).max().unwrap_or(0) + 1;
        w.floating.push(place);
        w.zoomed = false;
        let id = pane.id;
        pane.window = window;
        if (pane.cols, pane.rows) != (place.cols, place.rows) {
            pane.cols = place.cols;
            pane.rows = place.rows;
            if let Err(e) = pane.handle.resize(place.cols, place.rows) {
                eprintln!("grofe server: resizing {id}: {e}");
            }
        }
        self.panes.insert(id, pane);
        if let Some(w) = self.windows.get_mut(&window) {
            w.set_active(id);
        }
        Ok(())
    }

    /// A hidden scratchpad's process exited: it leaves the history, unless
    /// `generation` is not its process's (a respawn's event).
    // ponytail: no `pane-exited` is emitted for a hidden pane that exits.
    pub(crate) fn drop_stashed(&mut self, pane: PaneId, generation: u64) -> bool {
        let Some(at) = self
            .scratch
            .iter()
            .position(|s| s.pane.id == pane && s.pane.handle.generation() == generation)
        else {
            return false;
        };
        let gone = self.scratch.remove(at);
        self.note_closed(&gone.pane, None);
        true
    }
}

/// A hidden scratchpad's exit: its `pane-closed` goes out now.
pub(crate) fn hidden_exited(
    state: &mut crate::server::State,
    pane: PaneId,
    generation: u64,
    kind: &crate::server::pane::PaneEventKind,
) {
    if matches!(kind, crate::server::pane::PaneEventKind::Exited(..))
        && state.model.drop_stashed(pane, generation)
    {
        crate::server::hooks::run_pending(state);
        state.events.flush(&state.model);
    }
}
