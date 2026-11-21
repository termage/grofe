use super::*;

/// `switch-client -T`: client `id`'s key table.
pub(crate) fn set_key_table(state: &mut State, id: ClientId, table: &str) {
    if let Some(c) = state.clients.get_mut(&id) {
        c.keys.set_table(table);
        c.dirty = true;
    }
}

/// Shows `text` as it is (styles in it are not drawn, as tmux shows a
/// command's error) on client `id`'s status line for `display-time`.
pub fn show_message(state: &mut State, id: ClientId, text: &str) {
    show_message_as(state, id, text, render::Level::Error);
}

/// [`show_message`] for a message that is not an error.
pub fn show_message_as(state: &mut State, id: ClientId, text: &str, level: render::Level) {
    log_message(state, id, text);
    set_message(state, id, status::escape_hashes(text), None, false, level);
}

/// The status-line message area (spec §4.10) for `display-message`
/// without `-p` (TC.9): `text` is already expanded, and `#[...]` styles in
/// it are drawn, as tmux draws a `display-message`. It replaces the first
/// status row (tmux's `message-line 0`) for `display-time`, or until a key
/// is pressed; the rows below stay.
/// `delay` overrides `display-time` (`display-message -d`).
pub fn display_message(
    state: &mut State,
    id: ClientId,
    text: &str,
    delay: Option<Duration>,
    ignore_keys: bool,
    level: render::Level,
) {
    log_message(state, id, text);
    set_message(state, id, text.to_string(), delay, ignore_keys, level);
}

/// How many messages `show-messages` keeps when `message-limit` is unset:
/// tmux's default.
pub(super) const MESSAGE_LIMIT: usize = 1000;

/// Adds a status-line message for client `id` to the server's message
/// log, worded as tmux's `status_message_set` logs it.
pub(super) fn log_message(state: &mut State, id: ClientId, text: &str) {
    let Some(name) = state.clients.get(&id).map(|c| c.name.clone()) else {
        return;
    };
    push_message(state, format!("{name} message: {text}"));
}

/// Logs to `show-messages`, once per option and server, that setting `name`
/// does nothing: grofe stores it and never reads it.
pub(crate) fn warn_inert_option(state: &mut State, name: &str) {
    if state.registry.inert_warned.insert(name.to_string()) {
        push_message(state, format!("{name}: stored, but has no effect in grofe"));
    }
}

pub(super) fn push_message(state: &mut State, line: String) {
    let limit = state
        .model
        .options
        .lookup("message-limit", &[])
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(MESSAGE_LIMIT);
    let log = &mut state.registry.messages;
    log.push_back((SystemTime::now(), line));
    while log.len() > limit {
        log.pop_front();
    }
}

/// Plans the log lines of a CLI request's commands, as tmux 3.7b's
/// `cmdq_add_message` logs each command as it fires (`<client> command:
/// <cmd>`, re-printed with its full name, recorded): one line per command
/// of `argv`, with how many native commands it parsed to, so
/// [`log_fired`] logs it as the first of those runs. `who` is the
/// requesting client. Empty when the request does not parse, as tmux
/// fires nothing then, or when its commands cannot be told apart.
/// The log lines of the request now running: how many native commands each
/// covers, and its line. Logged as the commands fire ([`log_fired`]).
pub(crate) type CommandLog = VecDeque<(usize, Option<String>)>;

// ponytail: only CLI requests are logged, and not the rest held behind a
// wait. tmux also logs the commands of bindings (`<client> key <key>:
// <cmd>`), hooks and sourced files, and names a CLI run from a terminal by
// its tty.
pub(crate) fn plan_command_log(
    state: &State,
    who: &str,
    ctx: &ClientCtx,
    argv: &[String],
    parsed: usize,
) -> CommandLog {
    let mut plan = VecDeque::new();
    if argv.first().is_some_and(|verb| verb.starts_with("__")) {
        return plan;
    }
    for cmd in cmd::parse::split_argv(argv) {
        // Parsed again only to count, so the front end notes nothing.
        let parsed_again =
            crate::compat::quietly(|| crate::server::dispatch::parse_split(state, ctx, &cmd));
        let Ok(count) = parsed_again.map(|c| c.len()) else {
            return VecDeque::new();
        };
        let printed = match ctx.dialect {
            crate::protocol::Dialect::Tmux => tmux_print(&cmd),
            crate::protocol::Dialect::Grofe => None,
        };
        let printed = printed.unwrap_or_else(|| {
            let words: Vec<String> = cmd.iter().map(|w| quote::tmux_arg(w)).collect();
            words.join(" ")
        });
        plan.push_back((count, Some(format!("{who} command: {printed}"))));
    }
    if plan.iter().map(|(n, _)| n).sum::<usize>() != parsed {
        return VecDeque::new();
    }
    plan
}

/// `cmd` as `show-messages` prints a command it ran (tmux's `cmd_print`).
pub(super) fn print_for_log(dialect: crate::protocol::Dialect, cmd: &[String]) -> String {
    let printed = match dialect {
        crate::protocol::Dialect::Tmux => tmux_print(cmd),
        crate::protocol::Dialect::Grofe => None,
    };
    printed.unwrap_or_else(|| {
        let words: Vec<String> = cmd.iter().map(|w| quote::tmux_arg(w)).collect();
        words.join(" ")
    })
}

/// Logs `cmd`, a command a key's binding runs in a branch of its own
/// (`if-shell`), as run by that key.
pub(crate) fn log_key_command(state: &mut State, ctx: &ClientCtx, cmd: &[String]) {
    let Some(key) = ctx.request.event_key.clone() else {
        return;
    };
    let Some(who) = ctx
        .client
        .and_then(|id| state.clients.get(&id))
        .map(|c| c.name.clone())
    else {
        return;
    };
    let printed = print_for_log(ctx.dialect, cmd);
    push_message(state, format!("{who} key {key}: {printed}"));
}

/// Logs the planned line of the request command whose first native
/// command is about to run ([`plan_command_log`]), and any before it that
/// ran as no command at all.
pub(crate) fn log_fired(state: &mut State, log: &mut CommandLog) {
    while let Some((left, line)) = log.front_mut() {
        let (left, line) = (std::mem::take(left), line.take());
        if let Some(line) = line {
            push_message(state, line);
        }
        if left > 1 {
            if let Some(front) = log.front_mut() {
                front.0 = left - 1;
            }
            return;
        }
        log.pop_front();
        if left == 1 {
            return;
        }
    }
}

/// A request's error, as tmux's `cmdq_error` logs it for a client.
pub(crate) fn log_error(state: &mut State, who: &str, err: &str) {
    push_message(state, format!("{who} message: {err}"));
}

/// A tmux command as tmux's `cmd_print` prints it: the full name, the
/// flags with no value together and sorted, then each flag with a value,
/// sorted, then the arguments, each quoted as `args_escape` quotes it.
/// `None` for a verb tmux does not have.
pub(super) fn tmux_print(argv: &[String]) -> Option<String> {
    let (name, args) = argv.split_first()?;
    let spec = crate::compat::tmux::resolve(name).ok()?;
    // `[-c working-directory]` takes a value, `[-dErx]` does not.
    let takes = |flag: char| {
        spec.usage.split("[-").skip(1).any(|part| {
            let mut chars = part.chars();
            chars.next() == Some(flag) && chars.next() == Some(' ')
        }) || (spec.name == "select-pane" && flag == 'P')
    };
    let mut bare: std::collections::BTreeMap<char, usize> = Default::default();
    let mut valued: std::collections::BTreeMap<char, Vec<&str>> = Default::default();
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        if arg == "--" {
            i += 1;
            break;
        }
        let Some(letters) = arg.strip_prefix('-').filter(|l| !l.is_empty()) else {
            break;
        };
        i += 1;
        for (at, flag) in letters.char_indices() {
            if !takes(flag) {
                *bare.entry(flag).or_default() += 1;
                continue;
            }
            let rest = &letters[at + flag.len_utf8()..];
            let value = if rest.is_empty() {
                let next = args.get(i).map(String::as_str);
                i += 1;
                next.unwrap_or_default()
            } else {
                rest
            };
            valued.entry(flag).or_default().push(value);
            break;
        }
    }
    let mut out = spec.name.to_string();
    if !bare.is_empty() {
        out.push_str(" -");
        for (flag, n) in bare {
            out.extend(std::iter::repeat_n(flag, n));
        }
    }
    for (flag, values) in valued {
        for v in values {
            out.push_str(&format!(" -{flag} {}", quote::tmux_arg(v)));
        }
    }
    for word in args.get(i..).unwrap_or_default() {
        out.push(' ');
        out.push_str(&quote::tmux_arg(word));
    }
    Some(out)
}

/// The message log (TC.9's `show-messages`), oldest first.
pub(crate) fn messages(state: &State) -> impl DoubleEndedIterator<Item = &(SystemTime, String)> {
    state.registry.messages.iter()
}

/// Client `id` redraws (`refresh-client`): only its status line with
/// `status_only`, else everything, as if it had just attached.
pub(crate) fn refresh(state: &mut State, id: ClientId, status_only: bool) {
    let Some(c) = state.clients.get_mut(&id) else {
        return;
    };
    if !status_only {
        c.outbox.prev = None;
    }
    c.dirty = true;
    request_render(state, Instant::now());
}

/// Client `id`'s status rows as last drawn, and whether they are at the
/// top of its screen (TC.9's `choose-client` preview copies them).
pub(crate) fn drawn_status(state: &State, id: ClientId) -> Option<(&[StatusRow], bool)> {
    let c = state.clients.get(&id)?;
    Some((
        &c.status_rows,
        c.status_at == 0 && !c.status_rows.is_empty(),
    ))
}

/// `suspend-client` (TC.9): client `id` is asked to stop itself with
/// SIGTSTP, as tmux 3.7b's MSG_SUSPEND; nothing is drawn for it until it
/// resumes.
pub(crate) fn suspend(state: &mut State, id: ClientId) {
    if let Some(c) = state.clients.get_mut(&id)
        && !c.suspended
        && c.suspend_tx.try_send(()).is_ok()
    {
        c.suspended = true;
    }
}

/// A client named on the command line (`-c` or `-t` of the client
/// verbs), else the current client ([`current_client`]).
pub(crate) fn target_client(
    state: &State,
    ctx: &ClientCtx,
    name: Option<&str>,
) -> Result<ClientId, String> {
    match name {
        Some(name) => client_by_name(state, name),
        None => current_client(state, ctx).ok_or_else(|| "no current client".to_string()),
    }
}

pub(super) fn set_message(
    state: &mut State,
    id: ClientId,
    text: String,
    delay: Option<Duration>,
    ignore_keys: bool,
    level: render::Level,
) {
    let now = Instant::now();
    let Some(session) = state.clients.get(&id).map(|c| c.session) else {
        return;
    };
    let time = delay.unwrap_or_else(|| display_time(&state.model, session));
    let until = (!time.is_zero()).then(|| now + time);
    if let Some(c) = state.clients.get_mut(&id) {
        c.message = Some((text, until, level));
        // As tmux: with a delay of 0 (until a key), `-N` does not change
        // whether keys take the message down.
        if until.is_some() {
            c.message_ignores_keys = ignore_keys;
        }
        c.dirty = true;
        request_render(state, now);
    }
}

/// Nerd Font round caps, a pill's ends when neither `message-pill-ends`
/// nor the theme's `@pill_l` and `@pill_r` give them.
const ROUND_ENDS: &str = "\u{e0b6}\u{e0b4}";

/// How a `level` message or prompt pill is drawn while `message-pill` is on
/// for `chain`; `None` draws tmux's full-width bar. The colour is the
/// theme's (`@theme_blue`, `@theme_yellow`, `@theme_red`, text in
/// `@theme_bg`), else a named one; the ends are `message-pill-ends`, else
/// the theme's `@pill_l` and `@pill_r`, else round caps.
pub(crate) fn pill_look(
    model: &Model,
    chain: &[Owner],
    level: render::Level,
) -> Option<render::PillLook> {
    let get = |name: &str| model.options.lookup(name, chain).filter(|v| !v.is_empty());
    if get("message-pill") == Some("off") {
        return None;
    }
    let ends = get("message-pill-ends")
        .map(str::to_string)
        .or_else(|| Some(format!("{}{}", get("@pill_l")?, get("@pill_r")?)))
        .unwrap_or_else(|| ROUND_ENDS.to_string());
    let (theme, _, _, _) = level.look();
    let named = render::PillLook::named(level, &ends);
    Some(match (get(theme), get("@theme_bg")) {
        (Some(colour), Some(text)) => render::PillLook {
            colour: colour.to_string(),
            text: text.to_string(),
            ends,
        },
        _ => named,
    })
}

/// How many columns of status line `line` a pill covers: up to where
/// `status-right` starts, so the right side stays; the whole row when the
/// line has no `status-right`.
pub(crate) fn pill_span(rows: &[StatusRow], line: usize, cols: u16) -> usize {
    let right = rows.get(line).and_then(|row| {
        row.ranges
            .iter()
            .find(|r| r.kind == RangeKind::Right && r.end > r.start)
            .map(|r| r.start)
    });
    usize::from(right.unwrap_or(cols).min(cols))
}

/// The session's `display-time`, or tmux's default; 0 means until a key.
pub(super) fn display_time(model: &Model, session: SessionId) -> Duration {
    model
        .options
        .lookup("display-time", &[Owner::Session(session)])
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(DISPLAY_TIME, Duration::from_millis)
}

/// What a click at client cell `(col, row)` hits on client `id`'s status
/// line, as last drawn: when it is on a status row, that row's index
/// among the status rows and the range under it, if any (TC.10 sets
/// `mouse_status_range` and `mouse_y` from them). Rows and ranges both
/// come from the last draw, so an option changed since then cannot move a
/// click onto another row's ranges.
pub fn status_at(
    state: &State,
    id: ClientId,
    col: u16,
    row: u16,
) -> Option<(u16, Option<RangeKind>)> {
    let c = state.clients.get(&id)?;
    let line = row.checked_sub(c.status_at)?;
    let drawn = c.status_rows.get(usize::from(line))?;
    Some((line, drawn.range_at(col).map(|r| r.kind.clone())))
}

/// Client `id`'s status line as last drawn, for a menu or popup placed by
/// `-x W` / `-y W` / `-y S` (tmux's `cmd_display_menu_get_pos`).
pub struct StatusSpot {
    /// How many rows the status line takes.
    pub lines: u16,
    /// `status-position top`.
    pub top: bool,
    /// Each window range: the status row it is on (0 is the first), its
    /// first column, and the window index it names, in drawing order.
    pub windows: Vec<(u16, u16, u32)>,
}

/// Client `id`'s [`StatusSpot`]; `None` with the status line off.
pub fn status_spot(state: &State, id: ClientId) -> Option<StatusSpot> {
    let c = state.clients.get(&id)?;
    let geom = status_geometry(&state.model, c.session, c.size.1);
    if geom.lines == 0 {
        return None;
    }
    let windows = c
        .status_rows
        .iter()
        .enumerate()
        .flat_map(|(line, row)| {
            row.ranges.iter().filter_map(move |r| match r.kind {
                RangeKind::Window(index) => Some((line as u16, r.start, index)),
                _ => None,
            })
        })
        .collect();
    Some(StatusSpot {
        lines: geom.lines,
        top: geom.top,
        windows,
    })
}

/// The range under a click at client cell `(col, row)` on client `id`'s
/// status line as last drawn ([`status_at`]).
pub fn status_range_at(state: &State, id: ClientId, col: u16, row: u16) -> Option<RangeKind> {
    status_at(state, id, col, row).and_then(|(_, range)| range)
}
