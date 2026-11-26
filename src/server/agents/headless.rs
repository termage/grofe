//! Headless agent records: a `grofe.system` job that shows as an agent
//! (sidebar, grid, tray, `agent list`) while it runs, with no pane behind it.
//! The record lives in the agent store under a pane id no pane has; the
//! surfaces ask `Agents::is_headless` where they would look the pane up.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::{AgentState, Change, Report, ReportState, valid_kind};
use crate::server::dispatch::agent;
use crate::server::{State, clients, events};
use crate::types::PaneId;

fn report(state: ReportState, msg: Option<String>) -> Report {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX));
    Report {
        state,
        kind: None,
        msg,
        blocked: None,
        session_id: None,
        ts,
    }
}

/// Checks the record a `system` call asks for; its running state.
pub(crate) fn check(kind: &str, state: Option<&str>) -> Result<AgentState, String> {
    if !valid_kind(kind) {
        return Err(format!("grofe.system: agent kind {kind:?} is not a name"));
    }
    match state.map(ReportState::parse) {
        None => Ok(AgentState::Working),
        Some(Some(ReportState::Set(s))) => Ok(s),
        Some(_) => Err(format!(
            "grofe.system: agent state {:?} is not a state",
            state.unwrap_or_default()
        )),
    }
}

/// Registers a record of `kind` called `label`, in `running` state, in place
/// of any earlier one with that label.
pub(crate) fn start(state: &mut State, kind: &str, label: &str, running: AgentState) -> PaneId {
    for old in state.agents.headless_labelled(label) {
        remove(state, old);
    }
    let pane = state.agents.insert_headless(kind, label, Instant::now());
    agent::apply(state, pane, report(ReportState::Set(running), None));
    pane
}

/// The job ended with `code`: `done` for 0, else `error`.
pub(crate) fn finish(state: &mut State, pane: PaneId, code: i32) {
    if !state.agents.is_headless(pane) {
        return;
    }
    let r = if code == 0 {
        report(ReportState::Set(AgentState::Done), None)
    } else {
        report(
            ReportState::Set(AgentState::Error),
            Some(format!("exit {code}")),
        )
    };
    agent::apply(state, pane, r);
}

/// Drops a headless record, as `agent-changed` with no new state.
pub(crate) fn remove(state: &mut State, pane: PaneId) {
    if !state.agents.is_headless(pane) {
        return;
    }
    let Some(old) = state.agents.remove(pane) else {
        return;
    };
    let change = Change {
        old: Some(old.state),
        new: None,
        kind: old.kind,
        session_changed: false,
    };
    events::emit_agent_changed(state, pane, &change);
    clients::mark_all_dirty(state);
}

/// Dismisses tray entry `id`; a finished headless record goes with the entry
/// that reported it.
pub(crate) fn dismiss(state: &mut State, id: u64) -> bool {
    let pane = state
        .notify
        .items()
        .iter()
        .find(|n| n.id == id)
        .map(|n| n.pane);
    let found = state.notify.dismiss(id);
    if let Some(pane) = pane
        && state
            .agents
            .get(pane)
            .is_some_and(|a| a.label.is_some() && a.state != AgentState::Working)
    {
        remove(state, pane);
    }
    found
}
