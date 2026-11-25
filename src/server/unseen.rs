//! `#{pane_unseen_changes}` (tmux's `PANE_UNSEENCHANGES`): a pane had
//! output while it was in a mode (copy mode, the clock, a chooser), until
//! it leaves its last mode.

use std::collections::BTreeSet;

use crate::server::State;
use crate::types::PaneId;

/// Whether `pane` is in any mode, as `#{pane_in_mode}` counts it.
fn in_mode(state: &State, pane: PaneId) -> bool {
    crate::server::clock::format_var(state, pane, "pane_in_mode").is_some()
        || crate::server::copy::format_var(state, pane, "pane_in_mode").as_deref() == Some("1")
        || crate::server::tree::format_var(state, pane, "pane_in_mode").is_some()
}

/// Forgets the panes that left their modes: before a command, which may
/// enter one again, and with new output.
pub(crate) fn sweep(state: &mut State) {
    let kept: BTreeSet<PaneId> = state
        .unseen
        .iter()
        .copied()
        .filter(|p| in_mode(state, *p))
        .collect();
    state.unseen = kept;
}

/// The panes in `dirty` had output: those in a mode have unseen changes.
pub(crate) fn output(state: &mut State, dirty: &BTreeSet<PaneId>) {
    sweep(state);
    let seen: Vec<PaneId> = dirty
        .iter()
        .copied()
        .filter(|p| in_mode(state, *p))
        .collect();
    state.unseen.extend(seen);
}

/// `#{pane_unseen_changes}` for `pane`.
pub(crate) fn format_var(state: &State, pane: PaneId) -> String {
    let unseen = state.unseen.contains(&pane) && in_mode(state, pane);
    u8::from(unseen).to_string()
}
