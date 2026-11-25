use super::*;

/// The client a command acts on: the requesting client, else the most
/// recently active client on the request's pane's session, else the most
/// recently active client.
pub(crate) fn current_client(state: &State, ctx: &ClientCtx) -> Option<ClientId> {
    if let Some(id) = ctx.client.filter(|id| state.clients.contains_key(id)) {
        return Some(id);
    }
    // The session tmux picks for a pane in a window linked into several.
    let session = ctx
        .pane
        .and_then(|p| state.model.panes.get(&p))
        .and_then(|p| state.model.link_session(p.window, None));
    state
        .clients
        .iter()
        .filter(|(_, c)| session.is_none_or(|s| c.session == s))
        .max_by_key(|(_, c)| c.activity)
        .map(|(id, _)| *id)
}

pub(crate) fn client_by_name(state: &State, name: &str) -> Result<ClientId, String> {
    client_named(state, name).ok_or_else(|| format!("can't find client: {name}"))
}

/// The client `name` names, as tmux's `cmd_find_client` finds it: by its
/// name or its tty, with or without `/dev/`, one trailing `:` ignored.
pub(crate) fn client_named(state: &State, name: &str) -> Option<ClientId> {
    let name = name.strip_suffix(':').unwrap_or(name);
    state
        .clients
        .iter()
        .find(|(_, c)| {
            c.name == name
                || c.tty
                    .as_deref()
                    .is_some_and(|t| t == name || t.strip_prefix("/dev/") == Some(name))
        })
        .map(|(id, _)| *id)
}

/// `attach-session` run as a command: only an attached client can (its
/// bindings, for example); it switches that client to the target session.
/// The CLI's `attach` is the attach client itself (spec §4.6).
pub(crate) fn attach_session(
    state: &mut State,
    ctx: &ClientCtx,
    c: AttachSession,
) -> Result<String, String> {
    let Some(id) = ctx.client.filter(|id| state.clients.contains_key(id)) else {
        // tmux resolves the target before it finds there is no terminal.
        check_target(&state.model, &c.target, ctx, c.nest, None)?;
        return Err("open terminal failed: not a terminal".to_string());
    };
    let session = select_target(&mut state.model, &c.target, ctx, false)?;
    if let (Some(dir), Some(s)) = (&c.cwd, state.model.sessions.get_mut(&session)) {
        s.cwd = ctx.cwd.join(dir);
    }
    if let Some(list) = &c.flags
        && let Some(client) = state.clients.get_mut(&id)
    {
        client.flags.apply(list);
    }
    if c.read_only
        && let Some(client) = state.clients.get_mut(&id)
    {
        client.flags.read_only = true;
        client.flags.ignore_size = true;
    }
    if c.detach_others {
        detach_session_with(state, session, Some(id), c.kill_parent);
    }
    show_session_with(state, id, session, !c.no_env);
    Ok(String::new())
}

/// `detach-client`: `-s` detaches every client on a session, `-t` one
/// client by name, neither the current client; `-a` detaches every client
/// but that one instead.
pub(crate) fn detach_client(
    state: &mut State,
    ctx: &ClientCtx,
    c: DetachClient,
) -> Result<String, String> {
    if let Some(target) = &c.session {
        // tmux 3.7b finds `-s` with CMD_FIND_CANFAIL: no such session is
        // not an error (recorded).
        if let Ok(session) = state.model.resolve_session(target, ctx) {
            detach_session_with(state, session, None, c.kill_parent);
        }
        return Ok(String::new());
    }
    let id = match &c.client {
        Some(name) => client_by_name(state, name)?,
        None => current_client(state, ctx).ok_or_else(|| "no current client".to_string())?,
    };
    if c.all_others {
        let others: Vec<ClientId> = state.clients.keys().copied().filter(|o| *o != id).collect();
        for other in others {
            detach_with(state, other, c.kill_parent);
        }
    } else {
        detach_with(state, id, c.kill_parent);
    }
    Ok(String::new())
}

/// `list-clients`, in client order; `-F` expands per client, with
/// `client_*` variables over the client's current pane's. The default line
/// is tmux 3.7b's (its `[user ...]` part only shows for another user's
/// client, which grofe never has).
/// `list-clients`: every client, or with `-t` those showing that
/// session, that `-f` keeps.
pub fn list_clients(state: &State, ctx: &ClientCtx, c: ListClients) -> Result<String, String> {
    let model = &state.model;
    let session = match &c.target {
        Some(target) => Some(model.resolve_session(target, ctx)?),
        None => None,
    };
    let format = c.format.unwrap_or_else(|| match ctx.dialect {
        crate::protocol::Dialect::Tmux => options::tmux_template(options::TMUX_LIST_CLIENTS),
        crate::protocol::Dialect::Grofe => {
            "#{client_name}: #{session_name} [#{client_width}x#{client_height} \
             #{client_termname}] (#{client_flags})"
                .to_string()
        }
    });
    let sort = sorting::Sort::new(&c.sort)?;
    let mut listed: Vec<(&ClientId, &ClientEntry)> = state.clients.iter().collect();
    sort.apply(&mut listed, |order, (ia, a), (ib, b)| {
        let first = match order {
            sorting::Order::Name => sorting::by_name(&a.name, &b.name),
            sorting::Order::Size => a.size.0.cmp(&b.size.0).then(a.size.1.cmp(&b.size.1)),
            sorting::Order::Creation => ia.cmp(ib),
            sorting::Order::Activity => b.activity.cmp(&a.activity),
            _ => std::cmp::Ordering::Equal,
        };
        first.then_with(|| sorting::by_name(&a.name, &b.name))
    });
    let mut out = String::new();
    for (_, client) in listed {
        if session.is_some_and(|s| s != client.session) {
            continue;
        }
        let active = active_pane(model, client.session);
        let client_vars = ClientVars::new(model, client, active.map(|p| FormatCtx::new(model, p)));
        let server_vars = active.map(|p| options::ServerVars::fresh(state, p, &client_vars));
        let vars: &dyn cmd::FormatVars = match &server_vars {
            Some(s) => s,
            None => &client_vars,
        };
        if let Some(filter) = &c.filter {
            let got = cmd::expand(filter, vars);
            if got.is_empty() || got == "0" {
                continue;
            }
        }
        out.push_str(&format!("{}\n", cmd::expand(&format, vars)));
    }
    Ok(out)
}
