use super::*;

/// `client_name` for a client.
// ponytail: tmux names a client by its tty; the attach hello carries no
// tty path, so grofe numbers them.
pub fn client_name(id: ClientId) -> String {
    format!("client-{}", id.0)
}

/// Accepts an attach hello: resolves the target session (a window or pane
/// target also selects it), refuses self-nesting without `--nest`, and
/// registers the client as the session's latest (spec D6: attaching is
/// activity).
pub fn attach(state: &mut State, req: AttachRequest) -> Result<Accepted, String> {
    let now = Instant::now();
    let req_created = req.created;
    let target = match &req.target {
        // A tmux attach reads `-t` the tmux way (`=name` exact, FE.1).
        Some(raw) if req.ctx.dialect == crate::protocol::Dialect::Tmux => {
            cmd::target::with_tmux_targets(|| cmd::target::parse_target(raw))
                .map_err(|e| e.to_string())?
        }
        Some(raw) => cmd::target::parse_target(raw).map_err(|e| e.to_string())?,
        None => Target::Current,
    };
    check_target(
        &state.model,
        &target,
        &req.ctx,
        req.nest,
        req.tty.as_deref(),
    )?;
    let session = select_target(&mut state.model, &target, &req.ctx, false)?;
    // `-c`: where the session's new windows start from now on.
    if let (Some(dir), Some(s)) = (&req.extras.cwd, state.model.sessions.get_mut(&session)) {
        s.cwd = req.ctx.cwd.join(dir);
    }
    if req.detach_others {
        detach_session_with(state, session, None, req.extras.kill_parent);
    }
    // `-r` makes the client read-only and have its size ignored; `-f` sets
    // flags as `refresh-client -f` does.
    let mut flags = ClientFlags::default();
    if let Some(list) = &req.extras.flags {
        flags.apply(list);
    }
    if req.extras.read_only {
        flags.read_only = true;
        flags.ignore_size = true;
    }

    let id = ClientId(state.registry.next_id);
    state.registry.next_id += 1;
    let (outbox, output) = Outbox::new();
    let (exit_tx, exit) = oneshot::channel();
    let (suspend_tx, suspend) = mpsc::channel(1);
    let ctx = ClientCtx {
        client: Some(id),
        ..req.ctx
    };
    // The tty comes from the client: one that is not a device path is
    // taken as none.
    let tty = req.tty.filter(|t| usable_tty(t));
    let name = match &tty {
        Some(tty) if ctx.dialect == crate::protocol::Dialect::Tmux => tty.clone(),
        _ => client_name(id),
    };
    if req.caps.colours.is_some() {
        state.model.terminal_colours = req.caps.colours;
    }
    state.clients.insert(
        id,
        ClientEntry {
            name,
            tty,
            pid: req.pid,
            session,
            ctx,
            size: clamp_size(req.size),
            pending_size: None,
            caps: terminal_features(&state.model, &req.term, req.caps),
            term: req.term,
            activity: now,
            activity_time: SystemTime::now(),
            created: SystemTime::now(),
            decoder: Decoder::new(),
            keys: KeyState::new(),
            focused: true,
            mouse_prev: None,
            mouse_last: None,
            message: None,
            message_ignores_keys: false,
            status_rows: Vec::new(),
            status_at: 0,
            held: VecDeque::new(),
            held_bytes: 0,
            holds: 0,
            held_full: false,
            viewport: (0, 0),
            pan: None,
            suspended: false,
            suspend_tx,
            flags,
            mouse_modes: None,
            dirty: true,
            info: ClientInfo::default(),
            toast_until: std::cell::Cell::new(None),
            outbox,
            exit: Some(exit_tx),
        },
    );
    // A client that ignores size does not size the windows, unless it is
    // the session's only one.
    if !flags.ignore_size || !state.registry.owners.contains_key(&session) {
        state.registry.owners.insert(session, SizeOwner::new(id));
    }
    touch_session(&mut state.model, session);
    if let Some(s) = state.model.sessions.get_mut(&session) {
        s.last_attached = Some(SystemTime::now());
    }
    sync_attached(state);
    changed(state);
    if !req.extras.no_env
        && let Some(client_env) = state.clients.get(&id).map(|c| c.ctx.env.clone())
    {
        hooks::update_environment(state, session, &client_env);
    }
    // As tmux 3.7b (recorded): the panes it focuses, a session change, a
    // resize, then the attach.
    hooks::track_focus(state);
    hooks::client_session_changed(state, id);
    hooks::client_resized(state, id);
    // As tmux: only attach-session fires it, not new-session's attach.
    if !req_created {
        hooks::client_attached(state, id);
    }
    if state.binary.as_ref().is_some_and(|b| b.replaced()) {
        let text = status::escape_hashes(crate::server::build::STALE);
        display_message(
            state,
            id,
            &text,
            Some(Duration::from_secs(10)),
            false,
            render::Level::Warning,
        );
    }
    if req.extras.open_settings {
        let _ = crate::server::settings::overlay::open(state, id);
    }
    Ok(Accepted {
        id,
        output,
        exit,
        suspend,
    })
}

/// Whether an attach hello's tty can name its client: a path under
/// `/dev/`, no longer than `PATH_MAX`, with no control characters (it
/// reaches `list-clients`, the message log and scripts that parse them).
pub(super) fn usable_tty(tty: &str) -> bool {
    tty.starts_with("/dev/")
        && tty.len() <= libc::PATH_MAX as usize
        && !tty.chars().any(char::is_control)
}

/// Everything attach refuses before it changes anything, in tmux's order:
/// no sessions, then the target (session, window, pane), then self-nesting
/// without `--nest`.
///
/// Self-nesting is two rules. In the tmux dialect it is tmux 3.7b's
/// `server_client_check_nested` (recorded): a client with `$TMUX` set
/// whose terminal `tty` is one of this server's panes' is refused, to any
/// session; any other client attaches, so a popup's `tmux attach -t` to
/// the popup's own session shows it nested in the popup (TP.10). grofe's
/// own rule stays TC.1's: from a pane, its own session is refused and
/// another is allowed.
pub(super) fn check_target(
    model: &Model,
    target: &Target,
    ctx: &ClientCtx,
    nest: bool,
    tty: Option<&str>,
) -> Result<SessionId, String> {
    if model.sessions.is_empty() {
        return Err("no sessions".to_string());
    }
    let session = model.resolve_session(target, ctx)?;
    if let Target::Window(..) | Target::Pane(_) = target {
        model.resolve_window(target, ctx)?;
    }
    if let Target::Pane(_) = target {
        model.resolve_pane(target, ctx)?;
    }
    if !nest && ctx.dialect == crate::protocol::Dialect::Tmux {
        let tmux_set = ctx.env.iter().any(|(k, v)| k == "TMUX" && !v.is_empty());
        let on_a_pane = tty.filter(|t| !t.is_empty()).is_some_and(|t| {
            model
                .panes
                .values()
                .any(|p| p.handle.tty_path() == Some(std::path::Path::new(t)))
        });
        if tmux_set && on_a_pane {
            return Err(crate::compat::TMUX_NESTED.to_string());
        }
        return Ok(session);
    }
    if !nest
        && let Some(pane) = ctx.pane
        && let Some(p) = model.panes.get(&pane)
        && model.has_link(session, p.window)
    {
        return Err(format!(
            "refusing to attach session {} from inside itself (use --nest to force)",
            model.sessions[&session].name
        ));
    }
    Ok(session)
}

/// The attach check (`protocol::ATTACH_CHECK`): `args` are the client's
/// terminal, then `attach-session`'s arguments, checked as the attach
/// hello will be, changing nothing.
pub fn attach_check(state: &State, ctx: &ClientCtx, args: &[String]) -> Result<String, String> {
    let (tty, args) = args
        .split_first()
        .map_or(("", args), |(t, a)| (t.as_str(), a));
    let argv: Vec<String> = std::iter::once("attach-session".to_string())
        .chain(args.iter().cloned())
        .collect();
    // In the request's dialect, so a tmux `attach -t =name` is exact (FE.1).
    let parsed = crate::compat::parse_split(ctx, &argv)?;
    match parsed.into_iter().next() {
        Some(cmd::Command::AttachSession(c)) => {
            check_target(&state.model, &c.target, ctx, c.nest, Some(tty)).map(|_| String::new())
        }
        _ => Err("attach-session: unexpected command".to_string()),
    }
}

/// Resolves an attach or switch target to its session, making a window or
/// pane target that session's current window (and pane), as tmux does.
pub(crate) fn select_target(
    model: &mut Model,
    target: &Target,
    ctx: &ClientCtx,
    keep_zoom: bool,
) -> Result<SessionId, String> {
    let session = model.resolve_session(target, ctx)?;
    if let Target::Window(..) | Target::Pane(_) = target {
        let window = model.resolve_window(target, ctx)?;
        if let Target::Pane(_) = target {
            let pane = model.resolve_pane(target, ctx)?;
            crate::server::dispatch::pane::switch_active(model, window, pane, keep_zoom);
        }
        model.select_window_in(session, window);
    }
    Ok(session)
}

/// Client `id` now shows `session` (attach-session and switch-client):
/// the session it leaves becomes its last session, and it takes the new
/// session's size at once (switching is activity, spec D6).
pub(crate) fn show_session(state: &mut State, id: ClientId, session: SessionId) {
    show_session_with(state, id, session, true);
}

/// [`show_session`], leaving the session's environment as it is unless
/// `update_env` (`switch-client -E`, `attach-session -E`).
pub(crate) fn show_session_with(
    state: &mut State,
    id: ClientId,
    session: SessionId,
    update_env: bool,
) {
    let Some(entry) = state.clients.get_mut(&id) else {
        return;
    };
    let old = entry.session;
    entry.activity = Instant::now();
    if old != session {
        entry.session = session;
        entry.viewport = (0, 0);
        state.model.client_last_session.insert(id, old);
        leave_session(state, id, old);
    }
    let ignores_size = state.clients.get(&id).is_some_and(|c| c.flags.ignore_size);
    if !ignores_size || !state.registry.owners.contains_key(&session) {
        state.registry.owners.insert(session, SizeOwner::new(id));
    }
    touch_session(&mut state.model, session);
    if let Some(s) = state.model.sessions.get_mut(&session) {
        s.last_attached = Some(SystemTime::now());
    }
    sync_attached(state);
    if update_env && let Some(client_env) = state.clients.get(&id).map(|c| c.ctx.env.clone()) {
        hooks::update_environment(state, session, &client_env);
    }
    hooks::track_focus(state);
    hooks::client_session_changed(state, id);
    check_unattached(state);
}

/// Session activity (tmux's `session_update_activity`): what makes it the
/// session a command without a pane or target runs against.
pub(super) fn touch_session(model: &mut Model, session: SessionId) {
    if let Some(s) = model.sessions.get_mut(&session) {
        s.activity = Instant::now();
    }
}

/// Recounts `session_attached` for every session.
pub(super) fn sync_attached(state: &mut State) {
    for s in state.model.sessions.values_mut() {
        s.attached = 0;
    }
    for c in state.clients.values() {
        if let Some(s) = state.model.sessions.get_mut(&c.session) {
            s.attached += 1;
        }
    }
}
