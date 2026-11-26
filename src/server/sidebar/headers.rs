//! The sidebar's rows: agents grouped by session, each group under a header
//! row that shows the most urgent state of its agents (blocked, then done,
//! then working, then idle).

use super::{Row, State, jobs};
use crate::cmd::expand;
use crate::server::agents::worktree::{self, Block};
use crate::server::agents::{Rank, rank};
use crate::server::dispatch::agent;
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

/// One session's agents.
struct Session {
    name: String,
    panes: Vec<PaneId>,
}

/// The header row over `panes`: the most urgent state among them.
fn header(state: &State, label: String, panes: impl Iterator<Item = PaneId>) -> Row {
    let panes: Vec<PaneId> = panes.collect();
    Row {
        pane: None,
        label,
        header: Some(Header {
            rank: most_urgent(panes.iter().filter_map(|p| state.agents.get(*p)).map(rank)),
            agents: panes.len(),
        }),
    }
}

/// Agents grouped by session, the client's own session first, then by the
/// most urgent agent in each; a header row opens each group, urgency order
/// inside it; jobs last. Sessions in a git worktree of one repo share a
/// header named after the repo, and their rows say their branch.
pub(super) fn rows(state: &State, client: ClientId) -> Vec<Row> {
    let own = state
        .clients
        .get(&client)
        .and_then(|c| state.model.sessions.get(&c.session))
        .map(|s| s.name.clone());
    let mut sessions: Vec<Session> = Vec::new();
    for pane in state.agents.by_urgency() {
        if !state.model.panes.contains_key(&pane) {
            continue;
        }
        let name = field(state, pane, "#{session_name}");
        match sessions.iter_mut().find(|s| s.name == name) {
            Some(s) => s.panes.push(pane),
            None => sessions.push(Session {
                name,
                panes: vec![pane],
            }),
        }
    }
    if let Some(at) = sessions.iter().position(|s| Some(&s.name) == own.as_ref()) {
        let first = sessions.remove(at);
        sessions.insert(0, first);
    }
    let infos: Vec<_> = sessions
        .iter()
        .map(|s| worktree::of_pane(state, s.panes[0]))
        .collect();
    let mut rows = Vec::new();
    for block in worktree::arrange(&infos) {
        match block {
            Block::One(n) => {
                let s = &sessions[n];
                rows.push(header(state, s.name.clone(), s.panes.iter().copied()));
                rows.extend(entries(state, s, own.as_ref(), None));
            }
            Block::Repo { repo, members } => {
                let all = members
                    .iter()
                    .flat_map(|n| sessions[*n].panes.iter().copied());
                rows.push(header(state, repo, all));
                for n in members {
                    let branch = infos[n]
                        .as_ref()
                        .filter(|i| i.linked)
                        .map(|i| i.branch.as_str());
                    rows.extend(entries(state, &sessions[n], own.as_ref(), branch));
                }
            }
        }
    }
    rows.extend(jobs::rows(state));
    rows
}

/// The rows of a session's agents: labelled by their window, after the
/// session's name when it is not the client's own, or by `branch` (and the
/// window, when the session has several).
fn entries<'a>(
    state: &'a State,
    s: &'a Session,
    own: Option<&String>,
    branch: Option<&'a str>,
) -> impl Iterator<Item = Row> + 'a {
    let prefix = if Some(&s.name) == own {
        String::new()
    } else {
        format!("{}:", s.name)
    };
    s.panes.iter().map(move |&pane| {
        let window = agent::pane_format(state, pane, "agent-sidebar-format");
        let label = match branch {
            Some(b) if s.panes.len() > 1 => format!("{b}:{window}"),
            Some(b) => b.to_string(),
            None => format!("{prefix}{window}"),
        };
        Row {
            pane: Some(pane),
            label,
            header: None,
        }
    })
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
