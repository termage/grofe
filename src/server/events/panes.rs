//! `pane-created` and `pane-closed`: found by comparing the model's panes with
//! the ones the last flush saw, so one site covers whichever command, process
//! exit or restore made the change.

use std::collections::BTreeMap;

use super::Events;
use crate::event::{Event, Place};
use crate::server::model::Model;
use crate::types::PaneId;

impl Events {
    /// Takes the pane snapshot `pane-created` and `pane-closed` measure
    /// against now, if a subscription names them and none is kept yet: a
    /// command in the same chunk as the `subscribe` is then reported.
    pub(crate) fn watch_panes(&mut self, model: &Model) {
        if self.panes.is_none() && self.names_panes() {
            self.panes = Some(pane_places(model));
        }
    }

    fn names_panes(&self) -> bool {
        self.subscribers
            .iter()
            .any(|s| !s.gone && (s.wants("pane-created") || s.wants("pane-closed")))
    }

    /// `pane-created` and `pane-closed`, the one emission site for both: what
    /// differs in the model since the last look, whichever command, process
    /// exit or restore made it. Nothing is kept while nobody names them, so a
    /// subscription reports from its first flush on.
    pub(super) fn track_panes(&mut self, model: &Model) {
        if !self.names_panes() {
            self.panes = None;
            return;
        }
        let now = pane_places(model);
        if let Some(before) = self.panes.take() {
            for (pane, place) in now.iter().filter(|(p, _)| !before.contains_key(p)) {
                let (Some(window), Some(session)) = (place.window, place.session) else {
                    continue;
                };
                let event = Event::PaneCreated {
                    pane: *pane,
                    window,
                    session,
                };
                self.emit(event);
            }
            // Where it was, so a window or session scope hears it too.
            for (pane, place) in before.iter().filter(|(p, _)| !now.contains_key(p)) {
                self.emit_at(Event::PaneClosed { pane: *pane }, *place);
            }
        }
        self.panes = Some(now);
    }
}

/// Where each pane of the model is.
fn pane_places(model: &Model) -> BTreeMap<PaneId, Place> {
    model
        .panes
        .iter()
        .map(|(id, p)| {
            let place = Place {
                window: Some(p.window),
                session: model.windows.get(&p.window).map(|w| w.session),
            };
            (*id, place)
        })
        .collect()
}
