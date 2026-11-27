//! Running jobs (`grofe.system` with `opts.agent`) as sidebar rows: they are
//! not agents, so they have no pane and nothing to jump to.

use super::{Agent, Rank, Row, State, detail, rank};

/// A `working` row for each running job, labelled `kind · label`.
pub(super) fn rows(state: &State) -> impl Iterator<Item = Row> + '_ {
    state.agents.jobs.iter().map(|j| Row {
        pane: None,
        label: format!("{} · {}", j.kind, j.label),
        header: None,
    })
}

/// What an entry shows: its state, its name line (after the glyph) and its
/// detail line. `agent` is the row's agent, `None` for a job.
pub(super) fn entry(agent: Option<&Agent>, row: &Row) -> (Rank, String, String) {
    if let Some(h) = &row.header {
        return super::headers::entry(h, row);
    }
    match agent {
        Some(a) => (
            rank(a),
            format!(" {} · {}", a.kind.as_deref().unwrap_or("agent"), row.label),
            detail(a),
        ),
        // A job's label has its kind in it already.
        None => (Rank::Working, format!(" {}", row.label), "working".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{lines, setup};
    use super::super::{SidebarOp, set};
    use crate::server::agents::headless;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_running_job_is_a_working_row_until_it_ends() {
        let (mut state, client, _p, _out) = setup(160);
        set(&mut state, client, SidebarOp::On).expect("on");
        let shown = |state: &_| lines(state, client, 160);
        headless::start(&mut state, 1, 7, "triage", "issue 7");
        let l = shown(&state);
        let at = l
            .iter()
            .position(|r| r.contains("triage · issue 7"))
            .expect("the job's row");
        assert!(l[at + 1].contains("working"), "{:?}", l[at + 1]);
        headless::finish(&mut state, 1, 7, 0, None);
        assert!(shown(&state).iter().all(|r| !r.contains("issue 7")));
    }
}
