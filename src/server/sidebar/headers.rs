//! The sidebar's rows: agents grouped by session, each group under a header
//! row that shows the most urgent state of its agents (blocked, then done,
//! then working, then idle).

use super::{Row, State, jobs};
use crate::cmd::expand;
use crate::server::agents::{Rank, rank};
use crate::server::dispatch::options::{FormatCtx, ServerVars};
use crate::types::{ClientId, PaneId};

/// What a session's header row shows.
pub(super) struct Header {
    pub rank: Rank,
    pub agents: usize,
}

/// Position of `r` in the roll-up: lower is more urgent. A done agent that
/// has not been seen counts as done.
fn urgency(r: Rank) -> u8 {
    match r {
        Rank::Blocked => 0,
        Rank::Error => 1,
        Rank::Unseen | Rank::Done => 2,
        Rank::Working => 3,
        Rank::Idle => 4,
    }
}

/// The most urgent of `ranks` (idle for none).
fn most_urgent(ranks: impl Iterator<Item = Rank>) -> Rank {
    ranks.min_by_key(|r| urgency(*r)).unwrap_or(Rank::Idle)
}

/// `selected`, or the nearest row that is no header: the first one in the
/// direction of `down`, else the first one in the other.
pub(super) fn off_header(rows: &[Row], selected: usize, down: bool) -> usize {
    let at = selected.min(rows.len().saturating_sub(1));
    if rows.get(at).is_none_or(|r| r.header.is_none()) {
        return at;
    }
    let after = (at + 1..rows.len()).find(|i| rows[*i].header.is_none());
    let before = (0..at).rev().find(|i| rows[*i].header.is_none());
    let (first, second) = if down {
        (after, before)
    } else {
        (before, after)
    };
    first.or(second).unwrap_or(at)
}

/// A header's name line (after the glyph) and its detail line.
pub(super) fn entry(h: &Header, row: &Row) -> (Rank, String, String) {
    let word = match h.rank {
        Rank::Unseen => "done",
        r => r.as_str(),
    };
    let count = match h.agents {
        1 => "1 agent".to_string(),
        n => format!("{n} agents"),
    };
    (h.rank, format!(" {} · {word}", row.label), count)
}

fn field(state: &State, pane: PaneId, format: &str) -> String {
    let vars = FormatCtx::new(&state.model, pane);
    expand(format, &ServerVars::fresh(state, pane, &vars))
}

/// Agents grouped by session, the client's own session first, then by the
/// most urgent agent in each; a header row opens each group, urgency order
/// inside it; jobs last.
pub(super) fn rows(state: &State, client: ClientId) -> Vec<Row> {
    let own = state
        .clients
        .get(&client)
        .and_then(|c| state.model.sessions.get(&c.session))
        .map(|s| s.name.clone());
    let mut groups: Vec<(String, Vec<PaneId>)> = Vec::new();
    for pane in state.agents.by_urgency() {
        if !state.model.panes.contains_key(&pane) {
            continue;
        }
        let session = field(state, pane, "#{session_name}");
        match groups.iter_mut().find(|(s, _)| *s == session) {
            Some((_, panes)) => panes.push(pane),
            None => groups.push((session, vec![pane])),
        }
    }
    if let Some(at) = groups.iter().position(|(s, _)| Some(s) == own.as_ref()) {
        let first = groups.remove(at);
        groups.insert(0, first);
    }
    groups
        .into_iter()
        .flat_map(|(session, panes)| {
            let prefix = if Some(&session) == own.as_ref() {
                String::new()
            } else {
                format!("{session}:")
            };
            let header = Row {
                pane: None,
                label: session,
                header: Some(Header {
                    rank: most_urgent(panes.iter().filter_map(|p| state.agents.get(*p)).map(rank)),
                    agents: panes.len(),
                }),
            };
            let entries = panes.into_iter().map(move |pane| Row {
                pane: Some(pane),
                label: format!("{prefix}{}", field(state, pane, "#{window_name}")),
                header: None,
            });
            std::iter::once(header).chain(entries)
        })
        .chain(jobs::rows(state))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roll_up_follows_blocked_done_working_idle() {
        let top = |r: &[Rank]| most_urgent(r.iter().copied());
        assert_eq!(top(&[Rank::Idle, Rank::Working, Rank::Done]), Rank::Done);
        assert_eq!(
            top(&[Rank::Idle, Rank::Unseen, Rank::Working]),
            Rank::Unseen
        );
        assert_eq!(top(&[Rank::Idle, Rank::Working]), Rank::Working);
        assert_eq!(
            top(&[Rank::Done, Rank::Blocked, Rank::Working]),
            Rank::Blocked
        );
        assert_eq!(top(&[]), Rank::Idle);
    }
}
