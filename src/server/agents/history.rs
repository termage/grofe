//! The messages an agent's reports carried, for `grofe.agent.history`.

use std::collections::VecDeque;

use super::Agents;
use crate::types::PaneId;

/// Messages kept per agent for `grofe.agent.history`.
// ponytail: 50 messages per agent, oldest dropped; no option, and a record
// that is removed takes its history with it.
pub(super) const HISTORY_CAP: usize = 50;

/// Appends `msg`, dropping the oldest one past the cap.
pub(super) fn push(history: &mut VecDeque<String>, msg: String) {
    if history.len() == HISTORY_CAP {
        history.pop_front();
    }
    history.push_back(msg);
}

impl Agents {
    /// The last `n` messages of `pane`, oldest first.
    pub(crate) fn history(&self, pane: PaneId, n: usize) -> Vec<String> {
        let Some(a) = self.map.get(&pane) else {
            return Vec::new();
        };
        a.history
            .iter()
            .skip(a.history.len().saturating_sub(n))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::agents::{AgentState, Report, ReportState};
    use std::time::Instant;

    const P: PaneId = PaneId(1);

    fn set(state: AgentState, ts: u64) -> Report {
        Report {
            state: ReportState::Set(state),
            kind: None,
            msg: None,
            blocked: None,
            session_id: None,
            ts,
        }
    }

    #[test]
    fn history_keeps_the_last_messages_up_to_the_cap() {
        let mut ag = Agents::default();
        let now = Instant::now();
        for i in 1..=(HISTORY_CAP as u64 + 5) {
            let mut r = set(AgentState::Working, i);
            r.msg = Some(format!("m{i}"));
            ag.apply(P, r, true, now);
        }
        assert_eq!(ag.history(P, 2), ["m54", "m55"]);
        assert_eq!(ag.history(P, 999).len(), HISTORY_CAP);
        assert_eq!(ag.history(P, 999)[0], "m6");
        assert!(ag.history(PaneId(9), 3).is_empty());
    }

    #[test]
    fn history_takes_every_report_that_carried_a_message() {
        let mut ag = Agents::default();
        let now = Instant::now();
        let mut say = |state, ts, msg: Option<&str>| {
            let mut r = set(state, ts);
            r.msg = msg.map(String::from);
            ag.apply(P, r, true, now)
        };
        say(AgentState::Working, 1, Some("A"));
        say(AgentState::Done, 2, None);
        say(AgentState::Working, 3, Some("A"));
        assert_eq!(ag.history(P, 9), ["A", "A"]);
    }
}
