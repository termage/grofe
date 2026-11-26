//! Server-side agent entry point: the one place reports, bells and jumps
//! reach the agent store (spec §2.2 to §2.6, §5.2).

use std::time::{Duration, Instant};

use crate::cmd::expand;
use crate::config::Resolved;
use crate::protocol::ClientCtx;
use crate::server::agents::{AgentState, Change, Report, ReportState, Source, kinds};
use crate::server::dispatch::options::{FormatCtx, ServerVars};
use crate::server::notify::{Raised, desktop};
use crate::server::resume::Binding;
use crate::server::{State, clients, events, hooks, resume};
use crate::types::{ClientId, PaneId};

/// Whether a client is known to be unfocused: its terminal reported losing
/// focus and `focus-events` is on. With `focus-events` off its focus is
/// unknown and counts as focused (spec §2.4); a client that never reported
/// starts focused.
fn known_unfocused(state: &State, c: &clients::ClientEntry) -> bool {
    !c.focused && state.model.options.lookup("focus-events", &[]) == Some("on")
}

/// Whether `pane` is the active pane of the current window of an attached
/// client whose outer terminal is focused (a client whose focus is unknown
/// counts as focused, spec §2.4).
pub(crate) fn is_seen(state: &State, pane: PaneId) -> bool {
    let Some(window) = state.model.panes.get(&pane).map(|p| p.window) else {
        return false;
    };
    if state
        .model
        .windows
        .get(&window)
        .is_none_or(|w| w.active != pane)
    {
        return false;
    }
    state.clients.values().any(|c| {
        !known_unfocused(state, c)
            && state
                .model
                .sessions
                .get(&c.session)
                .is_some_and(|s| s.active == window)
    })
}

/// Applies a report to `pane`'s record: creates it bound to the pane's
/// foreground process, raises `agent-changed`, redraws, and sets the pane's
/// resume when the session id changed.
pub(crate) fn apply(state: &mut State, pane: PaneId, r: Report) {
    apply_from(state, pane, r, Source::Report);
}

/// [`apply`] for a report that came from `source`.
pub(crate) fn apply_from(state: &mut State, pane: PaneId, r: Report, source: Source) {
    if !state.agents_enabled || !state.model.panes.contains_key(&pane) {
        return;
    }
    let now = Instant::now();
    // A record whose process is gone is replaced, not updated (spec §2.3).
    let created = r.state != ReportState::Clear
        && state
            .agents
            .get(pane)
            .is_none_or(|a| a.binding.is_some_and(|b| !b.alive()));
    if created {
        let binding = foreground_binding(state, pane);
        let kind = r.kind.clone().unwrap_or_default();
        state.agents.ensure(pane, &kind, binding, now);
    }
    let seen = is_seen(state, pane);
    let before = state.agents.get(pane).map(|a| (a.state, a.since));
    let change = state.agents.apply(pane, r, seen, now);
    state.agents.set_source(pane, source);
    // A record this report made had no state before it.
    let change = match (change, created) {
        (Some(c), true) => Some(Change { old: None, ..c }),
        (None, true) => state.agents.get(pane).map(|a| Change {
            old: None,
            new: Some(a.state),
            kind: a.kind.clone(),
            session_changed: false,
            msg: None,
        }),
        (c, false) => c,
    };
    let Some(change) = change else {
        return;
    };
    if change.session_changed && change.new.is_some() {
        set_resume(state, pane);
    }
    announce(state, pane, &change, before, now);
}

/// Raises `agent-changed`, offers the change to the notification store and
/// redraws. `before` is the record's state and `since` before the change.
pub(crate) fn announce(
    state: &mut State,
    pane: PaneId,
    change: &Change,
    before: Option<(AgentState, Instant)>,
    now: Instant,
) {
    events::emit_agent_changed(state, pane, change);
    let worked = match (change.old, before) {
        (Some(AgentState::Working), Some((AgentState::Working, since))) => {
            now.saturating_duration_since(since)
        }
        _ => Duration::ZERO,
    };
    raise(state, pane, change, worked, now);
    clients::mark_all_dirty(state);
}

/// Offers a state change to the notification store (spec §6.1) and delivers
/// what it raises.
fn raise(state: &mut State, pane: PaneId, change: &Change, worked: Duration, now: Instant) {
    let Some(uid) = state.model.panes.get(&pane).map(|p| p.uid.clone()) else {
        return;
    };
    let msg = state
        .agents
        .get(pane)
        .and_then(|a| a.msg.clone().or_else(|| a.blocked.clone()));
    let configured = change
        .kind
        .as_deref()
        .is_some_and(|k| state.agent_kinds.iter().any(|x| x.name == k));
    let seen = is_seen(state, pane);
    let raised = state.notify.on_change(
        pane,
        &uid,
        change.kind.as_deref(),
        change.old.map(AgentState::as_str),
        change.new.map(AgentState::as_str),
        msg.as_deref(),
        worked,
        seen,
        configured,
        now,
    );
    if let Some(raised) = raised {
        emit_notify(state, raised);
        deliver(state, raised);
        clients::mark_all_dirty(state);
    }
}

/// The `notify` event for a raised notification (protocol v1 §6).
fn emit_notify(state: &mut State, raised: Raised) {
    let Some(n) = state.notify.items().iter().find(|n| n.id == raised.id) else {
        return;
    };
    let Some(pane) = n.pane else {
        return;
    };
    let session = state
        .model
        .panes
        .get(&pane)
        .and_then(|p| state.model.windows.get(&p.window))
        .map(|w| w.session);
    let event = crate::event::Event::Notify {
        pane,
        session,
        kind: n.kind.clone(),
        state: n.state.clone(),
        msg: n.msg.clone(),
    };
    state.events.emit(event);
}

/// Sends a raised notification out of the terminal (spec §6.4): `auto` is
/// OSC 9 when some client is known unfocused and the command when no client
/// is attached.
pub(crate) fn deliver(state: &mut State, raised: Raised) {
    if !raised.desktop {
        return;
    }
    let Some(n) = state.notify.items().iter().find(|n| n.id == raised.id) else {
        return;
    };
    let Some(pane) = n.pane else {
        return;
    };
    let state_name = n.state.clone();
    let title = format!("grofe: {}", n.kind.as_deref().unwrap_or("agent"));
    let label = pane_label(state, pane);
    let message = match &n.msg {
        Some(m) => format!("{label} {state_name}: {m}"),
        None => format!("{label} {state_name}"),
    };
    let delivery = state.notify.cfg.delivery.as_str();
    let (terminal, command) = match delivery {
        "terminal" => (true, false),
        "command" => (false, true),
        "auto" => (
            state.clients.values().any(|c| known_unfocused(state, c)),
            state.clients.is_empty(),
        ),
        _ => (false, false),
    };
    if terminal {
        let bytes = desktop::osc9(&format!("{title}: {message}"));
        let ids: Vec<ClientId> = state.clients.keys().copied().collect();
        for id in ids {
            clients::queue_raw(state, id, &bytes);
        }
    }
    // A test server (desktop::BUILTIN_OFF_ENV) runs a configured command
    // only, never the desktop's own notifier.
    if command && (state.notify.cfg.command.is_some() || !desktop::builtin_off()) {
        let os = if cfg!(target_os = "macos") {
            desktop::Os::Mac
        } else {
            desktop::Os::Linux
        };
        let cfg = &state.notify.cfg;
        let argv = desktop::argv(
            os,
            &title,
            &message,
            cfg.sound,
            cfg.command.as_deref(),
            &pane.to_string(),
            false,
        );
        state.notify_runner.submit(argv);
    }
}

pub(crate) fn pane_label(state: &State, pane: PaneId) -> String {
    let vars = FormatCtx::new(&state.model, pane);
    expand(
        "#{session_name}:#{window_index}.#{pane_index}",
        &ServerVars::fresh(state, pane, &vars),
    )
}

/// A bell or OSC 9/777 on a pane that never reported (spec §3.3).
pub(crate) fn attention(state: &mut State, pane: PaneId, msg: Option<String>) {
    if state.agents_enabled && state.agents.attention(pane, msg) {
        clients::mark_all_dirty(state);
    }
}

/// Reads `[agents]`: the built-in kinds with `[agents.kinds.<name>]` laid
/// over them key by key; `enabled = false` drops a kind.
pub(crate) fn configure(state: &mut State, r: &Resolved) {
    state.agents_enabled = r.agents.enabled;
    let mut all = kinds::builtin();
    for (name, c) in &r.agents.kinds {
        if c.enabled == Some(false) {
            all.retain(|k| &k.name != name);
            continue;
        }
        let at = match all.iter().position(|k| &k.name == name) {
            Some(at) => at,
            None => {
                all.push(kinds::Kind {
                    name: name.clone(),
                    matches: vec![name.clone()],
                    command: None,
                    resume: None,
                    worktree: None,
                });
                all.len() - 1
            }
        };
        let kind = &mut all[at];
        if let Some(m) = &c.matches {
            kind.matches = m.clone();
        }
        if c.command.is_some() {
            kind.command = c.command.clone();
        }
        if c.resume.is_some() {
            kind.resume = c.resume.clone();
        }
        if c.worktree.is_some() {
            kind.worktree = c.worktree.clone();
        }
    }
    state.agent_kinds = all;
    let config_dir = super::options::config_env().config_dir;
    let (files, errors) = crate::server::agents::detect::load(config_dir.as_deref());
    state.agents.detect = files;
    for e in errors {
        clients::log_error(state, "agents", &e);
    }
    super::notify::configure(state, r);
}

/// Takes the jumping client to `pane` (spec §5.2): the client that ran the
/// command, else the most recently active one. It stays in its session when
/// that holds the pane's window (linked windows included), else switches to
/// the pane's session; then the window and pane are selected.
pub(crate) fn jump(state: &mut State, ctx: &ClientCtx, pane: PaneId) -> Result<(), String> {
    let window = state
        .model
        .panes
        .get(&pane)
        .map(|p| p.window)
        .ok_or_else(|| format!("can't find pane: {pane}"))?;
    let client = jumping_client(state, ctx);
    let own = client
        .and_then(|id| state.clients.get(&id))
        .map(|c| c.session);
    let session = state
        .model
        .link_session(window, own)
        .ok_or_else(|| format!("pane {pane} is in no session"))?;
    state.model.select_window_in(session, window);
    super::pane::switch_active(&mut state.model, window, pane, false);
    if let Some(id) = client.filter(|_| own != Some(session)) {
        clients::show_session_with(state, id, session, false);
    }
    clients::mark_all_dirty(state);
    Ok(())
}

fn jumping_client(state: &State, ctx: &ClientCtx) -> Option<ClientId> {
    ctx.client
        .filter(|id| state.clients.contains_key(id))
        .or_else(|| {
            state
                .clients
                .iter()
                .max_by_key(|(_, c)| c.activity)
                .map(|(id, _)| *id)
        })
}

/// The binding for `pane`'s foreground process, if it has one.
///
/// A report typed in a plain shell is made by the short-lived `grofe` (or
/// `tmux`) CLI, which is then the foreground leader; the record binds to the
/// pane's root process (the shell) instead, or the sweep would drop it.
fn foreground_binding(state: &State, pane: PaneId) -> Option<Binding> {
    let p = state.model.panes.get(&pane)?;
    let group = p.handle.foreground_group()?;
    let argv = crate::server::pane::procinfo::leader(group).argv;
    let root = p.handle.pid().and_then(|pid| i32::try_from(pid).ok());
    Binding::of(bind_pid(group, argv.as_deref(), root))
}

/// The process a new record binds to: the foreground group, unless its
/// leader is the reporter itself and the pane's root process is known.
fn bind_pid(group: i32, leader_argv: Option<&[String]>, root: Option<i32>) -> i32 {
    let reporter = leader_argv
        .and_then(|a| a.first())
        .and_then(|a0| a0.rsplit('/').next())
        .is_some_and(|b| b == "grofe" || b == "tmux");
    match root {
        Some(root) if reporter => root,
        _ => group,
    }
}

/// Sets `pane`'s resume from its kind's template and session id (spec
/// §2.6), bound to the record's process. `argv[0]` is resolved against the
/// pane's own `PATH` as `pane-set-resume` does, and stays bare when that
/// finds nothing.
fn set_resume(state: &mut State, pane: PaneId) {
    let Some(a) = state.agents.get(pane) else {
        return;
    };
    let (Some(kind), Some(id)) = (a.kind.as_deref(), a.session_id.as_deref()) else {
        return;
    };
    let Some(argv) = state
        .agent_kinds
        .iter()
        .find(|k| k.name == kind)
        .and_then(|k| k.resume.as_deref())
        .and_then(|t| kinds::resume_argv(t, id))
    else {
        return;
    };
    let argv = resume::resolve_for_pane(state, pane, argv);
    let binding = a.binding.or_else(|| foreground_binding(state, pane));
    let Some(p) = state
        .model
        .panes
        .get_mut(&pane)
        .filter(|p| p.dead.is_none())
    else {
        return;
    };
    let changed = p.resume.as_ref() != Some(&argv) || p.expect.is_some();
    p.resume = Some(argv);
    p.expect = None;
    p.resume_binding = binding;
    if changed {
        hooks::resume_changed(state, pane);
    }
    resume::watch(state);
}

#[cfg(test)]
mod tests;
