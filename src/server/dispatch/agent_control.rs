//! `agent prompt|read|explain` (spec A2b) and screen detection: the commands
//! over an agent pane that type into it, read it and say why it has its state.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::cmd::{AgentPrompt, Resolve, Target};
use crate::lua::api::{Payload, PromptJob};
use crate::protocol::{ClientCtx, Failure};
use crate::server::agents::screen::Read;
use crate::server::agents::waiters::{Reply, Timer, Waiter};
use crate::server::agents::{AgentState, Change, ReportState, Source};
use crate::server::pane::CaptureExtra;
use crate::server::{State, clients, lua_async, lua_host};
use crate::types::{CaptureBound, CaptureOpts, PaneId};

use super::agent;

/// Lines `agent read` prints without `-n`.
const READ_LINES: usize = 50;
/// How long a wait lasts without `--timeout`.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3600);
/// States `--until` means by default.
const DEFAULT_UNTIL: [AgentState; 3] = [AgentState::Idle, AgentState::Blocked, AgentState::Done];

/// How a wait ended.
enum Outcome {
    Reached(AgentState),
    Timeout,
    /// The pane is gone.
    Closed,
    /// The agent left; the pane is still open.
    Left,
}

/// The states of `--until`, or the default ones when none are named.
fn parse_until(names: &[String]) -> Result<Vec<AgentState>, String> {
    if names.is_empty() {
        return Ok(DEFAULT_UNTIL.to_vec());
    }
    names
        .iter()
        .map(|n| {
            match ReportState::parse(n) {
                Some(ReportState::Set(s)) => Some(s),
                _ => None,
            }
            .ok_or_else(|| format!("unknown agent state in --until: {n}"))
        })
        .collect()
}

fn agent_pane(state: &State, ctx: &ClientCtx, target: &Target) -> Result<PaneId, String> {
    let pane = state.model.resolve_pane(target, ctx)?;
    if state.agents.get(pane).is_none() {
        return Err(format!("pane {pane} is not an agent pane"));
    }
    Ok(pane)
}

/// `text` without control characters, newline and tab apart: the text is
/// untrusted (issue comments), and an escape or a ^C typed into an agent is
/// not text.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| matches!(c, '\n' | '\t') || !c.is_control())
        .collect()
}

/// Types `text` and Enter into `pane` and no other pane (synchronize-panes
/// does not copy it): one bracketed paste when the pane has that on, else
/// the text on one line.
fn type_text(state: &State, pane: PaneId, text: &str) -> Result<(), String> {
    let p = state
        .model
        .panes
        .get(&pane)
        .filter(|_| clients::takes_input(state, pane))
        .ok_or_else(|| format!("pane {pane} takes no input"))?;
    let text = clean(text);
    let modes = p.handle.modes();
    let mut bytes = if modes.bracketed_paste {
        crate::server::input::encode_paste(text.as_bytes(), true, true, &modes)
    } else {
        text.replace(['\n', '\t'], " ").into_bytes()
    };
    bytes.push(b'\r');
    p.handle
        .enqueue_input(bytes)
        .map_err(|_| format!("input queue full for pane {pane}"))
}

/// Ends a wait with `outcome`.
fn settle(state: &mut State, w: Waiter, outcome: Outcome) {
    drop(w.timer);
    match w.reply {
        Reply::Cli(tx) => {
            let failed = |err: String, status| {
                Err(Failure {
                    err,
                    status: Some(status),
                    ..Failure::default()
                })
            };
            let _ = tx.send(match outcome {
                Outcome::Reached(s) => Ok(format!("{}\n", s.as_str())),
                Outcome::Timeout => failed("timed out waiting for the agent".to_string(), 1),
                Outcome::Closed => failed(format!("pane {} closed", w.pane), 2),
                Outcome::Left => failed(format!("agent left pane {}", w.pane), 2),
            });
        }
        Reply::Lua { generation, id } => {
            let payload = match outcome {
                Outcome::Reached(s) => Payload::Line(s.as_str().to_string()),
                _ => Payload::None,
            };
            let job = lua_async::guarded(generation, move |state| {
                lua_host::call_once(state, id, payload)
            });
            if state.jobs.try_send(job).is_err() {
                lua_host::release_once(state, generation, id);
                lua_host::log(
                    state,
                    "lua: grofe.agent.prompt: job queue full, callback dropped".into(),
                );
            }
        }
    }
}

/// An agent record of `pane` changed (see [`Change`]): a wait ends when the
/// state itself changed to one it wants (a report that only brings a message
/// or a session id does not), or when the record went (`Closed` when the pane
/// is gone too, else `Left`).
pub(crate) fn state_changed(state: &mut State, pane: PaneId, change: &Change) {
    let new = change.new;
    let moved = change.old != new;
    let done = state.agents.take_waiters(|w| {
        (w.pane == pane && new.is_none_or(|s| moved && w.until.contains(&s)))
            || matches!(&w.reply, Reply::Cli(tx) if tx.is_closed())
    });
    let gone = !state.model.panes.contains_key(&pane);
    for w in done {
        let outcome = match new {
            Some(s) => Outcome::Reached(s),
            None if gone => Outcome::Closed,
            None => Outcome::Left,
        };
        settle(state, w, outcome);
    }
}

/// Panes that left the model end their waits.
pub(crate) fn panes_closed(state: &mut State) {
    if state.agents.waiters.is_empty() {
        return;
    }
    let model = &state.model;
    let done = state
        .agents
        .take_waiters(|w| !model.panes.contains_key(&w.pane));
    for w in done {
        settle(state, w, Outcome::Closed);
    }
}

fn expire(state: &mut State, token: u64) {
    for w in state.agents.take_waiters(|w| w.token == token) {
        settle(state, w, Outcome::Timeout);
    }
}

/// Registers a wait for `pane` to reach one of `until`, ending with a timeout
/// after `timeout`.
fn wait(state: &mut State, pane: PaneId, until: Vec<AgentState>, timeout: Duration, reply: Reply) {
    let token = state.agents.add_waiter(pane, until, reply);
    let jobs = state.jobs.clone();
    // Without a runtime (a unit test) nothing times out.
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        let task = rt.spawn(async move {
            tokio::time::sleep(timeout).await;
            let job: crate::server::Job = Box::new(move |state| expire(state, token));
            let _ = jobs.send(job).await;
        });
        state.agents.set_timer(token, Timer(task.abort_handle()));
    }
}

/// The text was typed into `pane`: its agent is `working` (announced as any
/// state change), before a wait starts. A pane the screen can judge is armed:
/// output from before the text is not read, and its work ends as `idle` only
/// once the output since has rested.
fn mark_prompted(state: &mut State, pane: PaneId) {
    let now = Instant::now();
    let before = state.agents.get(pane).map(|a| (a.state, a.since));
    let change = state.agents.mark_working(pane, now);
    if state.agents_enabled {
        state.agents.arm(pane);
    }
    if let Some(change) = change {
        agent::announce(state, pane, &change, before, now);
    }
}

/// `agent prompt`: types the text, and with `--wait` answers when the agent
/// reaches a state of `--until`.
pub(crate) fn prompt(state: &mut State, ctx: &ClientCtx, c: AgentPrompt) -> Result<String, String> {
    let until = parse_until(&c.until)?;
    let pane = agent_pane(state, ctx, &c.target)?;
    type_text(state, pane, &c.text)?;
    mark_prompted(state, pane);
    if c.wait {
        let timeout = c.timeout_secs.map_or(DEFAULT_TIMEOUT, Duration::from_secs);
        let tx = super::defer_reply(state);
        wait(state, pane, until, timeout, Reply::Cli(tx));
    }
    Ok(String::new())
}

/// `grofe.agent.prompt`: as [`prompt`] with a wait, ending in Lua callback
/// `id` of runtime `generation` (with the state, or nothing on a timeout or a
/// closed pane).
pub(crate) fn prompt_lua(
    state: &mut State,
    ctx: &ClientCtx,
    req: &PromptJob,
    (generation, id): (u64, u32),
) -> Result<(), String> {
    let until = parse_until(&req.until)?;
    let target = crate::cmd::target::parse_target(&req.pane).map_err(|e| e.to_string())?;
    let pane = agent_pane(state, ctx, &target)?;
    type_text(state, pane, &req.text)?;
    mark_prompted(state, pane);
    let timeout = req
        .timeout_ms
        .map_or(DEFAULT_TIMEOUT, Duration::from_millis);
    wait(state, pane, until, timeout, Reply::Lua { generation, id });
    Ok(())
}

/// A Lua callback was cancelled: its wait goes.
pub(crate) fn cancel_lua(state: &mut State, generation: u64, id: u32) {
    state.agents.take_waiters(
        |w| matches!(w.reply, Reply::Lua { generation: g, id: i } if g == generation && i == id),
    );
}

/// A reload: the Lua waits go with the runtime.
pub(crate) fn clear_lua(state: &mut State) {
    state
        .agents
        .take_waiters(|w| matches!(w.reply, Reply::Lua { .. }));
}

/// The last `n` lines of `pane`: its alternate screen when it is on one,
/// else scrollback and screen; blank lines at the bottom do not count.
pub(crate) fn read_lines(state: &State, pane: PaneId, n: usize) -> Result<String, String> {
    let p = state
        .model
        .panes
        .get(&pane)
        .ok_or_else(|| format!("can't find pane: {pane}"))?;
    if n == 0 {
        return Ok(String::new());
    }
    // Reads from `want` lines above the screen (a bound clamps to the
    // history), and from twice as far when blank lines at the bottom left
    // fewer than `n`.
    let (mut want, mut prev) = (n, None);
    loop {
        let opts = CaptureOpts {
            start: Some(CaptureBound::Line(-i32::try_from(want).unwrap_or(i32::MAX))),
            end: Some(CaptureBound::VisibleEnd),
            escapes: false,
        };
        let all = p.handle.capture_rows(&opts, false, false, false, extra());
        let lines: Vec<&str> = all.trim_end().lines().collect();
        if lines.len() >= n || prev.as_ref() == Some(&all) {
            return Ok(lines[lines.len().saturating_sub(n)..].join("\n"));
        }
        prev = Some(all);
        want = want.saturating_mul(2);
    }
}

fn extra() -> CaptureExtra {
    CaptureExtra {
        numbered: false,
        flags: false,
        escaped: false,
        links: false,
    }
}

/// `agent read`: what [`read_lines`] gives, ended by a newline.
pub(crate) fn read(
    state: &State,
    ctx: &ClientCtx,
    target: &Target,
    lines: Option<usize>,
) -> Result<String, String> {
    let pane = state.model.resolve_pane(target, ctx)?;
    let text = read_lines(state, pane, lines.unwrap_or(READ_LINES))?;
    Ok(if text.is_empty() { text } else { text + "\n" })
}

/// `agent explain`: the kind, the state and where the state came from.
pub(crate) fn explain(state: &State, ctx: &ClientCtx, target: &Target) -> Result<String, String> {
    let pane = agent_pane(state, ctx, target)?;
    let Some(a) = state.agents.get(pane) else {
        return Err(format!("pane {pane} is not an agent pane"));
    };
    let mut out = format!(
        "kind: {}\nstate: {}\n",
        a.kind.as_deref().unwrap_or("unknown"),
        a.state.as_str()
    );
    match &a.source {
        None => out.push_str("source: none (no report or screen match yet)\n"),
        Some(Source::Report) => out.push_str("source: report\n"),
        Some(Source::Osc) => out.push_str("source: osc\n"),
        Some(Source::Bell) => out.push_str("source: bell\n"),
        Some(Source::Prompt) => out.push_str("source: prompt\n"),
        Some(Source::Screen { file, rule }) => {
            out.push_str(&format!("source: screen\nfile: {file}\nrule: {rule}\n"));
        }
    }
    Ok(out)
}

/// How long a pane's output must rest before its screen is read, and the
/// longest a busy pane waits: a screen drawn in several writes is read whole.
const SETTLE: Duration = Duration::from_millis(100);
const SETTLE_MAX: Duration = Duration::from_millis(500);

/// Output on `panes`: those with an agent that detection can judge are read
/// once their output has settled (see [`scan_pending`]).
pub(crate) fn note_output(state: &mut State, panes: &BTreeSet<PaneId>) {
    if !state.agents_enabled || state.agents.detect.is_empty() {
        return;
    }
    let agents = &mut state.agents;
    let judged: Vec<PaneId> = panes
        .iter()
        .copied()
        .filter(|p| agents.screen_judged(*p))
        .collect();
    if judged.is_empty() {
        return;
    }
    let now = Instant::now();
    for pane in &judged {
        agents.output(*pane, now);
    }
    agents.scan.pending.extend(judged);
    let first = *agents.scan.first.get_or_insert(now);
    agents.scan.at = Some((now + SETTLE).min(first + SETTLE_MAX));
}

/// When the pending screens are read.
pub(crate) fn deadline(state: &State) -> Option<Instant> {
    state.agents.scan.at
}

/// Screen detection (A2): reads the screens of the panes with settled
/// output, and takes the state their last lines show (see [`Agents::judge`]).
/// A record that has had a report is not judged.
pub(crate) fn scan_pending(state: &mut State, now: Instant) {
    if state.agents.scan.at.is_none_or(|at| at > now) {
        return;
    }
    state.agents.scan.at = None;
    state.agents.scan.first = None;
    for pane in std::mem::take(&mut state.agents.scan.pending) {
        let Some(file) = state
            .agents
            .get(pane)
            .and_then(|a| a.kind.as_ref())
            .and_then(|k| state.agents.detect.get(k))
        else {
            continue;
        };
        let Some(p) = state.model.panes.get(&pane) else {
            continue;
        };
        let opts = CaptureOpts {
            start: None,
            end: None,
            escapes: false,
        };
        let screen = p.handle.capture_rows(&opts, false, false, false, extra());
        let (hit, name) = (file.detect(&screen), file.source.clone());
        match state.agents.judge(pane, hit, now) {
            Read::Skip => {}
            Read::Later(due) => {
                let agents = &mut state.agents;
                agents.scan.pending.insert(pane);
                agents.scan.at = Some(agents.scan.at.map_or(due, |at| at.min(due)));
            }
            Read::Set(new, rule) => {
                let source = Source::Screen { file: name, rule };
                let before = state.agents.get(pane).map(|a| (a.state, a.since));
                let seen = agent::is_seen(state, pane);
                if let Some(change) = state.agents.screen(pane, new, source, seen, now) {
                    agent::announce(state, pane, &change, before, now);
                }
            }
        }
    }
}
