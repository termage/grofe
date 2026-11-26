//! Jobs a script asked to show (`grofe.system`'s `opts.agent`). They are not
//! agents: one entry per running job, in `Agents::jobs`, shown as `working`
//! rows by the sidebar and grid and dropped when the job ends.

use std::time::Instant;

use crate::server::{State, clients};

/// A running job. `generation` is the Lua runtime's generation and `id` the job's
/// callback id in it: together they name the job, as ids restart at a reload.
#[derive(Debug, Clone)]
pub(crate) struct Job {
    pub generation: u64,
    pub id: u32,
    pub kind: String,
    pub label: String,
    pub started: Instant,
}

pub(crate) fn start(state: &mut State, generation: u64, id: u32, kind: &str, label: &str) {
    state.agents.jobs.push(Job {
        generation,
        id,
        kind: kind.to_string(),
        label: label.to_string(),
        started: Instant::now(),
    });
    clients::mark_all_dirty(state);
}

fn take(state: &mut State, generation: u64, id: u32) -> Option<Job> {
    let at = state
        .agents
        .jobs
        .iter()
        .position(|j| j.generation == generation && j.id == id)?;
    clients::mark_all_dirty(state);
    Some(state.agents.jobs.remove(at))
}

/// The job was cancelled: it just goes.
pub(crate) fn cancel(state: &mut State, generation: u64, id: u32) {
    take(state, generation, id);
}

/// A reload: every job goes.
pub(crate) fn clear(state: &mut State) {
    if !state.agents.jobs.is_empty() {
        state.agents.jobs.clear();
        clients::mark_all_dirty(state);
    }
}

/// The job ended with `code`: it goes, and the tray hears `done` for 0, else
/// `error` (the usual notification rules apply, bar those about a pane).
pub(crate) fn finish(state: &mut State, generation: u64, id: u32, code: i32) {
    let Some(job) = take(state, generation, id) else {
        return;
    };
    let (new, msg) = match code {
        0 => ("done", None),
        c => ("error", Some(format!("exit {c}"))),
    };
    let worked = job.started.elapsed();
    let now = Instant::now();
    state
        .notify
        .on_job_end(&job.kind, &job.label, new, msg.as_deref(), worked, now);
}
