use super::*;

/// A client size within tmux's bounds and grofe's cell cap
/// ([`crate::types::cap_size`]).
pub(super) fn clamp_size((cols, rows): (u16, u16)) -> (u16, u16) {
    cap_size(cols, rows)
}

/// A frame from client `id`.
pub(crate) fn frame(state: &mut State, id: ClientId, frame: Frame) {
    match frame {
        Frame::Input(bytes) => input(state, id, &bytes, Instant::now()),
        Frame::Resize(cols, rows) => {
            crate::server::settings::overlay::resized(state, id, (cols, rows));
            let Some(c) = state.clients.get_mut(&id) else {
                return;
            };
            // A suspended client sends its size when it resumes: its
            // terminal is its own again, so everything is drawn anew.
            if c.suspended {
                c.suspended = false;
                c.outbox.prev = None;
                c.mouse_modes = None;
                c.dirty = true;
                sync_mouse_modes(state);
                let Some(c) = state.clients.get_mut(&id) else {
                    return;
                };
                c.pending_size = Some(clamp_size((cols, rows)));
                request_render(state, Instant::now());
                hooks::client_resized(state, id);
                return;
            }
            c.pending_size = Some(clamp_size((cols, rows)));
            request_render(state, Instant::now());
            hooks::client_resized(state, id);
        }
        Frame::Detach => detach(state, id),
        // Server-to-client kinds: ignored if a client sends them.
        Frame::Output(_) | Frame::Exit(_) | Frame::Suspend => {}
    }
}

/// Client `id`'s connection ended.
pub(crate) fn gone(state: &mut State, id: ClientId) {
    remove(state, id, None);
}

/// Detaches client `id` with tmux's message.
pub(crate) fn detach(state: &mut State, id: ClientId) {
    detach_with(state, id, false);
}

/// [`detach`]; with `kill_parent` (tmux's `MSG_DETACHKILL`) the parent of
/// the client's process, its shell, gets SIGHUP as the client goes, which
/// tmux's client does by itself.
pub(crate) fn detach_with(state: &mut State, id: ClientId, kill_parent: bool) {
    let reason = state
        .clients
        .get(&id)
        .map(|c| detached_from(&state.model, c.session));
    let parent = kill_parent
        .then(|| {
            let pid = state.clients.get(&id)?.pid?;
            crate::server::pane::procinfo::parent(pid)
        })
        .flatten();
    remove(state, id, reason);
    if let Some(parent) = parent.filter(|p| *p > 1) {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(parent),
            nix::sys::signal::Signal::SIGHUP,
        );
    }
}

pub(super) fn detached_from(model: &Model, session: SessionId) -> String {
    match model.sessions.get(&session) {
        Some(s) => format!("detached (from session {})", s.name),
        None => "detached".to_string(),
    }
}

/// Detaches every client on `session` except `keep`, hanging up their
/// parents with `kill_parent`.
pub(super) fn detach_session_with(
    state: &mut State,
    session: SessionId,
    keep: Option<ClientId>,
    kill_parent: bool,
) {
    let ids: Vec<ClientId> = state
        .clients
        .iter()
        .filter(|(id, c)| c.session == session && Some(**id) != keep)
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        detach_with(state, id, kill_parent);
    }
}

/// Removes a client; with a reason, it is told why (its connection writes
/// what is still pending, then the `exit` frame).
pub(super) fn remove(state: &mut State, id: ClientId, reason: Option<String>) {
    crate::server::passthrough::on_client_gone(state, id);
    let Some(mut c) = state.clients.remove(&id) else {
        return;
    };
    crate::server::prompt::client_gone(state, id);
    crate::server::menu::client_gone(state, id);
    crate::server::popup::client_gone(state, id);
    crate::server::tree::client_gone(state, id);
    crate::server::picker::client_gone(state, id);
    crate::server::sidebar::client_gone(state, id);
    crate::server::grid::client_gone(state, id);
    crate::server::keymenu::client_gone(state, id);
    crate::server::settings::overlay::client_gone(state, id, &c.ctx);
    crate::server::mouse::client_gone(state, id);
    if let (Some(tx), Some(reason)) = (c.exit.take(), reason) {
        let _ = tx.send(reason);
    }
    state.model.client_last_session.remove(&id);
    sync_attached(state);
    leave_session(state, id, c.session);
    graphics::on_clients_changed(state);
    hooks::track_focus(state);
    hooks::client_detached(state, &c.name, c.session);
    check_unattached(state);
}

/// tmux's `server_check_unattached`: a session no client shows goes when
/// its `destroy-unattached` says so (`on`; `keep-last` keeps the last of a
/// group and every ungrouped one; `keep-group` keeps the last of a group).
/// Run where a session loses or gains a client, and when the option is set.
pub(crate) fn check_unattached(state: &mut State) {
    use crate::server::dispatch::options::Owner;
    let unattached: Vec<SessionId> = state
        .model
        .sessions
        .values()
        .filter(|s| s.attached == 0)
        .map(|s| s.id)
        .collect();
    for id in unattached {
        let mode = state
            .model
            .options
            .lookup("destroy-unattached", &[Owner::Session(id)]);
        let grouped = state.model.group_of(id).map(|g| g.members.len());
        let destroy = match mode {
            Some("on") => true,
            Some("keep-last") => grouped.is_some_and(|n| n > 1),
            Some("keep-group") => grouped.is_none_or(|n| n > 1),
            _ => false,
        };
        if destroy {
            let removed = state.model.remove_session(id);
            state.events.panes_killed(removed);
        }
    }
    if state.model.closed() {
        state.exit = true;
    }
}

/// `id` no longer shows `session`: if it owned the size, the session's
/// most recently active remaining client takes over at once.
pub(super) fn leave_session(state: &mut State, id: ClientId, session: SessionId) {
    let Some(owner) = state.registry.owners.get_mut(&session) else {
        return;
    };
    if owner.pending.is_some_and(|(c, _)| c == id) {
        owner.pending = None;
    }
    if owner.client != id {
        return;
    }
    let next = state
        .clients
        .iter()
        .filter(|(cid, c)| c.session == session && **cid != id)
        .max_by_key(|(_, c)| c.activity)
        .map(|(cid, _)| *cid);
    match next {
        Some(next) => *owner = SizeOwner::new(next),
        None => {
            state.registry.owners.remove(&session);
        }
    }
    changed(state);
}

/// Tells every client why it is going away and forgets them (server exit).
pub(crate) fn shutdown(state: &mut State, reason: &str) {
    crate::server::popup::shutdown(state);
    for (_, mut c) in std::mem::take(&mut state.clients) {
        if let Some(tx) = c.exit.take() {
            let _ = tx.send(reason.to_string());
        }
    }
}

/// The session a client of the destroyed session `name` moves to, by its
/// `detach-on-destroy` `mode` (tmux's `server_destroy_session`): `off`, the
/// most recently active other session; `no-detached`, the most recent one
/// no client shows; `previous` and `next`, the neighbour in name order
/// (tmux 3.7b's crash on a NULL sort order is not copied). `on` detaches.
pub(super) fn successor(state: &State, name: &str, mode: &str) -> Option<SessionId> {
    let mut sessions: Vec<_> = state.model.sessions.values().collect();
    sessions.sort_by(|a, b| a.name.cmp(&b.name));
    let newest = |keep: &dyn Fn(&crate::server::model::Session) -> bool| {
        sessions
            .iter()
            .filter(|s| keep(s))
            .fold(None::<&&crate::server::model::Session>, |best, s| {
                best.filter(|b| s.activity <= b.activity).or(Some(s))
            })
            .map(|s| s.id)
    };
    // Where the destroyed session sat in name order.
    let before = sessions.iter().filter(|s| s.name.as_str() < name).count();
    match mode {
        "off" => newest(&|_| true),
        "no-detached" => newest(&|s| !state.clients.values().any(|c| c.session == s.id)),
        "previous" => before
            .checked_sub(1)
            .or(sessions.len().checked_sub(1))
            .map(|i| sessions[i].id),
        "next" => sessions.get(before).or(sessions.first()).map(|s| s.id),
        _ => None,
    }
}

/// After anything that may have changed what clients show (a command, a
/// pane exiting): clients of a session that is gone leave (as tmux's
/// `[exited]`), window sizes follow their owners, and every client redraws.
pub(crate) fn changed(state: &mut State) {
    let orphans: Vec<ClientId> = state
        .clients
        .iter()
        .filter(|(_, c)| !state.model.sessions.contains_key(&c.session))
        .map(|(id, _)| *id)
        .collect();
    let destroyed = std::mem::take(&mut state.model.destroyed_sessions);
    // One successor per destroyed session, chosen before any client moves,
    // as tmux picks `s_new` once: a second client must not see the first
    // one's session as attached.
    let successors: BTreeMap<SessionId, Option<SessionId>> = destroyed
        .iter()
        .map(|(id, (name, mode))| (*id, successor(state, name, mode)))
        .collect();
    for id in orphans {
        let next = state
            .clients
            .get(&id)
            .and_then(|c| successors.get(&c.session).copied().flatten());
        // `no-detach-on-destroy`: where `on` and `no-detached` would detach
        // the client, it goes to the session most recently used instead.
        let next = next.or_else(|| {
            let c = state
                .clients
                .get(&id)
                .filter(|c| c.flags.no_detach_on_destroy)?;
            let (name, mode) = destroyed.get(&c.session)?;
            matches!(mode.as_str(), "on" | "no-detached")
                .then(|| successor(state, name, "off"))
                .flatten()
        });
        match next {
            Some(session) => {
                show_session(state, id, session);
                state.model.client_last_session.remove(&id);
            }
            None => remove(state, id, Some("exited".to_string())),
        }
    }
    let live = &state.model.sessions;
    state.registry.owners.retain(|s, _| live.contains_key(s));
    // A killed pane's copy mode goes with it.
    let panes = &state.model.panes;
    state.copy.retain(|p| panes.contains_key(&p));
    sync_pane_status(&mut state.model);
    sync_pane_defaults(&state.model);
    apply_sizes(state);
    graphics::on_clients_changed(state);
    sync_mouse_modes(state);
    mark_all_dirty(state);
    arm_names(state, Instant::now());
}
