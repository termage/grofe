//! Calls of `agent prompt --wait` (and Lua's `grofe.agent.prompt`) waiting for
//! a pane's agent to reach a state.

use tokio::sync::oneshot;

use super::{AgentState, Agents};
use crate::protocol::Failure;
use crate::types::PaneId;

/// Who is told when the wait ends.
#[derive(Debug)]
pub(crate) enum Reply {
    /// The CLI's deferred reply.
    Cli(oneshot::Sender<Result<String, Failure>>),
    /// Lua callback `id` of runtime `generation`.
    Lua { generation: u64, id: u32 },
}

#[derive(Debug)]
pub(crate) struct Waiter {
    pub token: u64,
    pub pane: PaneId,
    pub until: Vec<AgentState>,
    pub reply: Reply,
}

impl Agents {
    pub(crate) fn add_waiter(&mut self, pane: PaneId, until: Vec<AgentState>, reply: Reply) -> u64 {
        self.waiter_seq += 1;
        let token = self.waiter_seq;
        self.waiters.push(Waiter {
            token,
            pane,
            until,
            reply,
        });
        token
    }

    /// Takes out the waiters `pick` accepts, and those whose CLI is gone.
    pub(crate) fn take_waiters(&mut self, mut pick: impl FnMut(&Waiter) -> bool) -> Vec<Waiter> {
        let (taken, kept) = std::mem::take(&mut self.waiters)
            .into_iter()
            .partition(|w| pick(w));
        self.waiters = kept;
        taken
    }
}
