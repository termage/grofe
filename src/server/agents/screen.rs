//! Screen detection's side of the store: the debounce of pane output, and
//! what a screen read does to a record.

use std::collections::BTreeSet;
use std::time::Instant;

use super::{AgentState, Agents, Change, Source};
use crate::types::PaneId;

/// Panes with output since their screens were last read, and when the next
/// read is due (see `agent_control::note_output`).
#[derive(Debug, Default)]
pub(crate) struct Scan {
    pub pending: BTreeSet<PaneId>,
    pub at: Option<Instant>,
    pub first: Option<Instant>,
}

impl Agents {
    /// Whether a screen that matches no rule means `idle` for `pane`: its
    /// state is `working` or `blocked` and came from the screen, so the screen
    /// no longer shows it.
    pub(crate) fn screen_lost(&self, pane: PaneId) -> bool {
        self.map.get(&pane).is_some_and(|a| {
            matches!(a.state, AgentState::Working | AgentState::Blocked)
                && matches!(a.source, Some(Source::Screen { .. }))
        })
    }

    /// The state the screen shows (never for a record that has had a report,
    /// which is fresher). `None` when nothing changed but the rule.
    pub(crate) fn screen(
        &mut self,
        pane: PaneId,
        state: AgentState,
        source: Source,
        seen: bool,
        now: Instant,
    ) -> Option<Change> {
        let a = self.map.get_mut(&pane).filter(|a| !a.reported)?;
        a.source = Some(source);
        if a.state == state {
            return None;
        }
        let old = a.state;
        a.state = state;
        a.since = now;
        a.blocked = None;
        a.msg = None;
        match state {
            AgentState::Working | AgentState::Idle => a.unseen = false,
            _ if !seen => a.unseen = true,
            _ => {}
        }
        Some(Change {
            old: Some(old),
            new: Some(state),
            kind: a.kind.clone(),
            session_changed: false,
            msg: None,
        })
    }
}
