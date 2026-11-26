use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use super::{AgentState, Agents, Change, Source};
use crate::types::PaneId;

/// How long a prompted pane's output must rest before its work can end as
/// `idle`.
pub(crate) const PROMPT_QUIET: Duration = Duration::from_secs(1);

/// Panes with output since their screens were last read, and when the next
/// read is due (see `agent_control::note_output`).
#[derive(Debug, Default)]
pub(crate) struct Scan {
    pub pending: BTreeSet<PaneId>,
    pub at: Option<Instant>,
    pub first: Option<Instant>,
    /// Panes `agent prompt` marked `working` that the screen can judge, with
    /// when they last printed output since the prompt (`None`: none yet).
    pub prompted: BTreeMap<PaneId, Option<Instant>>,
}

/// What a read of a pane's screen means.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Read {
    /// Nothing.
    Skip,
    /// Not yet: read again at this time.
    Later(Instant),
    /// The state and the rule that gave it.
    Set(AgentState, String),
}

impl Agents {
    /// Whether the screen of `pane` can be judged: its record has had no
    /// report and its kind has a detection file.
    pub(crate) fn screen_judged(&self, pane: PaneId) -> bool {
        self.map.get(&pane).is_some_and(|a| {
            !a.reported && a.kind.as_ref().is_some_and(|k| self.detect.contains_key(k))
        })
    }

    /// After `agent prompt`: the screen may end the work as `idle` only after
    /// output since the prompt has rested. Only for a pane the prompt marked
    /// `working` that the screen can judge; a second prompt starts over.
    pub(crate) fn arm(&mut self, pane: PaneId) -> bool {
        let prompted = self
            .map
            .get(&pane)
            .is_some_and(|a| a.state == AgentState::Working && a.source == Some(Source::Prompt));
        if !prompted || !self.screen_judged(pane) {
            return false;
        }
        self.scan.prompted.insert(pane, None);
        self.scan.pending.remove(&pane);
        true
    }

    /// Output on `pane`.
    pub(crate) fn output(&mut self, pane: PaneId, now: Instant) {
        if let Some(last) = self.scan.prompted.get_mut(&pane) {
            *last = Some(now);
        }
    }

    /// Whether a screen that matches no rule means `idle` for `pane`: its
    /// state is `working` or `blocked` and came from the screen or from
    /// `agent prompt`, so the screen no longer shows it.
    fn screen_lost(&self, pane: PaneId) -> bool {
        self.map.get(&pane).is_some_and(|a| {
            matches!(a.state, AgentState::Working | AgentState::Blocked)
                && matches!(a.source, Some(Source::Screen { .. } | Source::Prompt))
        })
    }

    /// What a read of `pane` that matched `hit` means at `now`. A screen that
    /// matches nothing changes nothing, except that a state the screen or the
    /// prompt gave falls back to `idle`. For a prompted pane `idle` needs
    /// output since the prompt and then `PROMPT_QUIET` without more; any
    /// other result ends the arm.
    pub(crate) fn judge(
        &mut self,
        pane: PaneId,
        hit: Option<(AgentState, String)>,
        now: Instant,
    ) -> Read {
        let (state, rule) = match hit {
            Some(hit) => hit,
            None if self.screen_lost(pane) => (AgentState::Idle, "no rule matched".to_string()),
            None => return Read::Skip,
        };
        if state == AgentState::Idle
            && let Some(last) = self.scan.prompted.get(&pane)
        {
            match last {
                None => return Read::Skip,
                Some(t) if now < *t + PROMPT_QUIET => return Read::Later(*t + PROMPT_QUIET),
                Some(_) => {}
            }
        }
        self.scan.prompted.remove(&pane);
        Read::Set(state, rule)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::agents::detect::DetectFile;
    use crate::server::agents::{Report, ReportState};

    const PANE: PaneId = PaneId(1);

    /// An agents table with a `fake` kind that has a detection file, and a
    /// prompted pane of that kind (or of `bare`, which has no file).
    fn prompted(kind: &str, t0: Instant) -> Agents {
        let mut ag = Agents::default();
        let file = DetectFile::parse("t", "working = \"WORK\"\nblocked = \"ASK\"\n").expect("file");
        ag.detect.insert("fake".to_string(), file);
        ag.ensure(PANE, kind, None, t0);
        ag.mark_working(PANE, t0).expect("working");
        ag
    }

    fn report(state: ReportState) -> Report {
        Report {
            state,
            kind: None,
            msg: None,
            blocked: None,
            session_id: None,
            ts: 1,
        }
    }

    #[test]
    fn a_prompt_to_a_pane_without_a_detection_file_arms_nothing() {
        let mut ag = prompted("bare", Instant::now());
        assert!(!ag.arm(PANE));
        assert!(ag.scan.prompted.is_empty());
        assert_eq!(ag.scan.at, None);
    }

    #[test]
    fn no_idle_before_output_since_the_prompt() {
        let t0 = Instant::now();
        let mut ag = prompted("fake", t0);
        assert!(ag.arm(PANE));
        let late = t0 + Duration::from_secs(5);
        assert_eq!(ag.judge(PANE, None, late), Read::Skip);
        assert!(ag.scan.prompted.contains_key(&PANE));
    }

    #[test]
    fn idle_after_one_second_of_quiet_with_no_rule_matching() {
        let t0 = Instant::now();
        let mut ag = prompted("fake", t0);
        assert!(ag.arm(PANE));
        ag.output(PANE, t0 + Duration::from_millis(10));
        let soon = t0 + Duration::from_millis(500);
        assert_eq!(
            ag.judge(PANE, None, soon),
            Read::Later(t0 + Duration::from_millis(10) + PROMPT_QUIET)
        );
        let quiet = t0 + Duration::from_millis(1100);
        assert_eq!(
            ag.judge(PANE, None, quiet),
            Read::Set(AgentState::Idle, "no rule matched".to_string())
        );
        assert!(ag.scan.prompted.is_empty());
    }

    #[test]
    fn a_working_rule_match_keeps_working_and_ends_the_arm() {
        let t0 = Instant::now();
        let mut ag = prompted("fake", t0);
        assert!(ag.arm(PANE));
        ag.output(PANE, t0);
        let hit = Some((AgentState::Working, "working = WORK".to_string()));
        let Read::Set(state, rule) = ag.judge(PANE, hit, t0) else {
            panic!("a match is set")
        };
        assert_eq!(state, AgentState::Working);
        let source = Source::Screen {
            file: "t".to_string(),
            rule,
        };
        assert!(ag.screen(PANE, state, source, true, t0).is_none());
        assert!(ag.scan.prompted.is_empty());
        // Now it is the screen's state: it falls back to idle at once.
        let later = t0 + Duration::from_millis(200);
        assert!(matches!(
            ag.judge(PANE, None, later),
            Read::Set(AgentState::Idle, _)
        ));
    }

    #[test]
    fn a_report_or_a_removed_record_clears_the_arm() {
        let t0 = Instant::now();
        let mut ag = prompted("fake", t0);
        assert!(ag.arm(PANE));
        ag.apply(
            PANE,
            report(ReportState::Set(AgentState::Working)),
            true,
            t0,
        );
        assert!(ag.scan.prompted.is_empty());
        let mut ag = prompted("fake", t0);
        assert!(ag.arm(PANE));
        ag.remove(PANE);
        assert!(ag.scan.prompted.is_empty());
    }

    #[test]
    fn a_second_prompt_starts_over() {
        let t0 = Instant::now();
        let mut ag = prompted("fake", t0);
        assert!(ag.arm(PANE));
        ag.output(PANE, t0);
        ag.scan.pending.insert(PANE);
        assert!(ag.arm(PANE));
        assert_eq!(ag.scan.prompted.get(&PANE), Some(&None));
        assert!(ag.scan.pending.is_empty());
    }
}
