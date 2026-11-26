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

/// A wait's timeout task, aborted when the wait goes.
#[derive(Debug)]
pub(crate) struct Timer(pub tokio::task::AbortHandle);

impl Drop for Timer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Debug)]
pub(crate) struct Waiter {
    pub token: u64,
    pub pane: PaneId,
    pub until: Vec<AgentState>,
    pub reply: Reply,
    pub timer: Option<Timer>,
}

impl Agents {
    /// Adds a wait, after dropping those whose CLI is gone.
    pub(crate) fn add_waiter(&mut self, pane: PaneId, until: Vec<AgentState>, reply: Reply) -> u64 {
        self.waiter_seq += 1;
        let token = self.waiter_seq;
        self.waiters
            .retain(|w| !matches!(&w.reply, Reply::Cli(tx) if tx.is_closed()));
        self.waiters.push(Waiter {
            token,
            pane,
            until,
            reply,
            timer: None,
        });
        token
    }

    /// Hands the wait `token` its timeout task.
    pub(crate) fn set_timer(&mut self, token: u64, timer: Timer) {
        if let Some(w) = self.waiters.iter_mut().find(|w| w.token == token) {
            w.timer = Some(timer);
        }
    }

    /// Takes out the waiters `pick` accepts.
    pub(crate) fn take_waiters(&mut self, mut pick: impl FnMut(&Waiter) -> bool) -> Vec<Waiter> {
        let (taken, kept) = std::mem::take(&mut self.waiters)
            .into_iter()
            .partition(|w| pick(w));
        self.waiters = kept;
        taken
    }
}
