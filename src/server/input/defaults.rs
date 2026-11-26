use super::*;

/// The default key table (spec §5.3, the "tmux default bindings" row, D10):
/// tmux 3.7b's stock `prefix` bindings for the commands grofe has, with
/// repeat where tmux has `-r` and tmux's notes (`list-keys -N`). New panes
/// and windows open in the active pane's cwd (spec §5.3), which tmux leaves
/// to a `-c '#{pane_current_path}'` in the user's config, and prompts and
/// confirmations spell tmux's `#P`/`#W` as native formats. `o` and `<` (the
/// window menu) are tmux's own rows in its dialect (their `+`/`-` target
/// offsets, FE.1's), `>` opens grofe's pane menu ([`grofe_pane_menu`]) on
/// the active pane and `C` opens grofe's settings menu, where tmux has
/// customize-mode. Copy mode binds `[`, `PPage` and the `copy-mode-vi` table.
pub(crate) fn default_bindings() -> KeyTables {
    let mut t = KeyTables::default();
    let argv = |words: &[&str]| vec![words.iter().map(|s| s.to_string()).collect()];
    for &(key, words, repeat, note) in PREFIX_ROWS {
        bind(
            &mut t,
            PREFIX_TABLE,
            key,
            argv(words),
            repeat,
            Some(note),
            Dialect::Grofe,
        );
    }
    for n in 0..=9 {
        let target = format!(":{n}");
        bind(
            &mut t,
            PREFIX_TABLE,
            &n.to_string(),
            argv(&["select-window", "-t", &target]),
            false,
            Some(&format!("Select window {n}")),
            Dialect::Grofe,
        );
    }
    for (table, rows) in [
        (COPY_MODE_VI_TABLE, COPY_MODE_VI),
        (COPY_MODE_TABLE, COPY_MODE_EMACS),
    ] {
        for (key, text) in rows {
            put(&mut t, table, key, text, None, Dialect::Grofe);
        }
    }
    // FE.1: stock `o` and `<`. Their `:.+` and `:-1` offsets have no
    // native spelling, so they are tmux's rows, run through the front end.
    // Neither repeats.
    for &(key, _, note, text) in TMUX_STOCK.iter().filter(|(k, ..)| ["o", "<"].contains(k)) {
        put(&mut t, PREFIX_TABLE, key, text, Some(note), Dialect::Tmux);
    }
    // grofe identity's pane menu on the active pane, below it, as tmux's
    // stock `>` places its own.
    let pane_menu = format!(
        "{GROFE_PANE_MENU_TITLE} -x P -y P {}",
        grofe_pane_menu_items(None)
    );
    put(
        &mut t,
        PREFIX_TABLE,
        ">",
        &pane_menu,
        Some("Display pane menu"),
        Dialect::Grofe,
    );
    // tmux 3.7b's stock mouse rows, in its dialect ([`NATIVE_STOCK_ROOT`]):
    // ordinary bindings, which the mouse module runs as they are (a border
    // drag, `resize-pane -M`, is the mouse's own).
    for (name, text) in TMUX_STOCK_ROOT {
        if NATIVE_STOCK_ROOT.contains(name) {
            t.insert_stock_root(name, text);
        }
    }
    let mouse_menu = grofe_pane_menu();
    for (key, text) in [
        // FR.4: grofe identity's pane menu, with Float and Unfloat; a server
        // with tmux identity binds tmux's stock menu instead.
        ("MouseDown3Pane", mouse_menu.as_str()),
        // FR.3: Ctrl and drag anywhere in a pane moves it, as dragging a
        // framed tile's title does; grofe's own, so a tmux-identity server
        // drops it and the key is unbound, as in tmux.
        ("C-MouseDrag1Pane", "drag-pane -M"),
        // The float control of a framed title's tab (`render::frame_tab`);
        // grofe's own, as the tab is.
        ("MouseDown1Control7", GROFE_FLOAT_CONTROL),
        // A click on the status line's agents badge opens the tray; any
        // other click switches client, as tmux's stock row does.
        ("MouseDown1Status", GROFE_STATUS_CLICK),
    ] {
        put(&mut t, ROOT_TABLE, key, text, None, Dialect::Grofe);
    }
    for (table, bindings) in t.tables.iter_mut() {
        for b in bindings.values_mut() {
            b.group = Some(default_group(table, &b.cmds).to_string());
        }
    }
    t
}

/// Binds `cmds` to `key` in `table` as a default binding, with its note,
/// in `dialect`.
fn bind(
    t: &mut KeyTables,
    table: &str,
    key: &str,
    cmds: Vec<Vec<String>>,
    repeat: bool,
    note: Option<&str>,
    dialect: Dialect,
) {
    if let Ok(key) = parse_key(key) {
        let mut b = Binding::new(cmds, repeat, BindingSource::Default);
        b.desc = note.map(str::to_string);
        b.dialect = dialect;
        t.insert(table, key, b);
    }
}

/// [`bind`] for `text`, a command string, that does not repeat.
fn put(
    t: &mut KeyTables,
    table: &str,
    key: &str,
    text: &str,
    note: Option<&str>,
    dialect: Dialect,
) {
    if let Ok(cmds) = crate::cmd::split_command_string(text) {
        bind(t, table, key, cmds, false, note, dialect);
    }
}

/// grofe's default `prefix` rows: the key, its argv, whether it repeats, its note.
#[rustfmt::skip]
const PREFIX_ROWS: &[(&str, &[&str], bool, &str)] = &[
    ("%", &["split-window", "-h", "-c", "#{pane_current_path}"], false, "Split window horizontally"),
    ("\"", &["split-window", "-c", "#{pane_current_path}"], false, "Split window vertically"),
    ("c", &["new-window", "-c", "#{pane_current_path}"], false, "Create a new window"),
    ("d", &["detach-client"], false, "Detach the current client"),
    ("n", &["next-window"], false, "Select the next window"),
    ("p", &["previous-window"], false, "Select the previous window"),
    ("l", &["last-window"], false, "Select the previously current window"),
    ("(", &["switch-client", "-p"], false, "Switch to the previous session"),
    (")", &["switch-client", "-n"], false, "Switch to the next session"),
    ("L", &["switch-client", "-l"], false, "Switch to the last session"),
    ("z", &["resize-pane", "-Z"], false, "Zoom the active pane"),
    ("C-b", &["send-prefix"], false, "Send the prefix key"),
    (":", &["command-prompt"], false, "Prompt for a command"),
    (",", &["command-prompt", "-I", "#W", "rename-window \"%%\""], false, "Rename current window"),
    ("$", &["command-prompt", "-I", "#S", "rename-session \"%%\""], false, "Rename current session"),
    // D10: the stock keys whose verbs are the structure verbs.
    (";", &["last-pane"], false, "Move to the previously active pane"),
    ("C-o", &["rotate-window"], false, "Rotate through the panes"),
    ("M-o", &["rotate-window", "-D"], false, "Rotate through the panes in reverse"),
    ("q", &["display-panes"], false, "Display pane numbers"),
    (".", &["command-prompt", "-T", "target", "move-window -t \"%%\""], false, "Move the current window"),
    ("'", &["command-prompt", "-T", "window-target", "-p", "index", "select-window -t \":%%\""], false, "Prompt for window index to select"),
    ("C-z", &["suspend-client"], false, "Suspend the current client"),
    ("D", &["choose-client", "-Z"], false, "Choose and detach a client from a list"),
    ("~", &["show-messages"], false, "Show messages"),
    ("E", &["select-layout", "-E"], false, "Spread panes out evenly"),
    ("M-n", &["next-window", "-a"], false, "Select the next window with an alert"),
    ("M-p", &["previous-window", "-a"], false, "Select the previous window with an alert"),
    ("/", &["command-prompt", "-k", "-p", "key", "list-keys -1N \"%%\""], false, "Describe key binding"),
    ("x", &["confirm-before", "-p", "kill-pane #{pane_index}? (y/n)", "kill-pane"], false, "Kill the active pane"),
    ("&", &["confirm-before", "-p", "kill-window #{window_name}? (y/n)", "kill-window"], false, "Kill current window"),
    ("!", &["break-pane"], false, "Break pane to a new window"),
    ("m", &["select-pane", "-m"], false, "Toggle the marked pane"),
    ("M", &["select-pane", "-M"], false, "Clear the marked pane"),
    ("t", &["clock-mode"], false, "Show a clock"),
    ("*", &["new-pane"], false, "New floating pane"),
    // grofe's own: tmux 3.7b binds no `e`.
    ("e", &["command-prompt", "-I", "#{pane_title}", "select-pane -T \"%%%\""], false, "Rename the active pane"),
    // grofe's own: tmux 3.7b has no key for `kill-server`.
    ("Q", &["confirm-before", "-p", "kill-server: stop grofe and every session? (y/n)", "kill-server"], false, "Stop the server and every session"),
    // grofe's own: tmux 3.7b binds no `u`.
    ("u", &["agent", "next"], false, "Jump to the agent that needs you"),
    // grofe's own: tmux 3.7b binds no `G`.
    ("G", &["copy-mode", "-h"], false, "Copy a match on screen by its label"),
    ("a", &["new-scratchpad"], false, "Open a scratchpad"),
    ("S", &["toggle-scratchpad"], false, "Hide or show a scratchpad"),
    ("A", &["choose-scratchpad"], false, "Choose a scratchpad from a list"),
    ("{", &["swap-pane", "-U"], false, "Swap the active pane with the pane above"),
    ("}", &["swap-pane", "-D"], false, "Swap the active pane with the pane below"),
    ("i", &["display-message"], false, "Display window information"),
    ("w", &["choose-tree", "-Zw"], false, "Choose a window from a list"),
    ("Space", &["key-menu", "-T", "prefix"], false, "Show the key menu"),
    // grofe's own: tmux identity keeps `C` for customize-mode.
    ("C", &["settings"], false, "Open the settings menu"),
    ("M-Space", &["next-layout"], false, "Select next layout"),
    ("U", &["choose-agent"], false, "Pick an agent"),
    ("b", &["agent-sidebar", "toggle"], false, "Toggle the agent sidebar"),
    ("O", &["agent-grid"], false, "Show all agents"),
    ("y", &["choose-notification"], false, "Show notifications"),
    ("?", &["list-keys", "-N"], false, "List key bindings"),
    ("r", &["refresh-client"], false, "Redraw the current client"),
    ("s", &["choose-tree", "-Zs"], false, "Choose a session from a list"),
    // TM.4: the buffer keys.
    ("]", &["paste-buffer", "-p"], false, "Paste the most recent paste buffer"),
    ("#", &["list-buffers"], false, "List all paste buffers"),
    ("=", &["choose-buffer", "-Z"], false, "Choose a paste buffer from a list"),
    ("-", &["delete-buffer"], false, "Delete the most recent paste buffer"),
    ("DC", &["refresh-client", "-c"], true, "Reset so the visible part of the window follows the cursor"),
    // FR.4: grofe's own, so a tmux-identity server drops them with the rest
    // of grofe's prefix defaults ([`KeyTables::use_tmux_stock`]) and gets
    // tmux's `f` (find-window) back. `f` is the main key; `F` stays for
    // configs that bind `f` themselves.
    ("f", &["if-shell", "-F", "#{pane_floating_flag}", "unfloat-pane", "float-pane"], false, "Float or unfloat the active pane"),
    ("F", &["if-shell", "-F", "#{pane_floating_flag}", "unfloat-pane", "float-pane"], false, "Float or unfloat the active pane"),
    ("Up", &["select-pane", "-U"], true, "Select the pane above the active pane"),
    ("M-Up", &["resize-pane", "-U", "5"], true, "Resize the pane up by 5"),
    ("C-Up", &["resize-pane", "-U"], true, "Resize the pane up"),
    ("Down", &["select-pane", "-D"], true, "Select the pane below the active pane"),
    ("M-Down", &["resize-pane", "-D", "5"], true, "Resize the pane down by 5"),
    ("C-Down", &["resize-pane", "-D"], true, "Resize the pane down"),
    ("Left", &["select-pane", "-L"], true, "Select the pane to the left of the active pane"),
    ("M-Left", &["resize-pane", "-L", "5"], true, "Resize the pane left by 5"),
    ("C-Left", &["resize-pane", "-L"], true, "Resize the pane left"),
    ("Right", &["select-pane", "-R"], true, "Select the pane to the right of the active pane"),
    ("M-Right", &["resize-pane", "-R", "5"], true, "Resize the pane right by 5"),
    ("C-Right", &["resize-pane", "-R"], true, "Resize the pane right"),
    ("S-Up", &["refresh-client", "-U", "10"], true, "Move the visible part of the window up"),
    ("S-Down", &["refresh-client", "-D", "10"], true, "Move the visible part of the window down"),
    ("S-Left", &["refresh-client", "-L", "10"], true, "Move the visible part of the window left"),
    ("S-Right", &["refresh-client", "-R", "10"], true, "Move the visible part of the window right"),
    ("M-1", &["select-layout", "even-horizontal"], false, "Set the even-horizontal layout"),
    ("M-2", &["select-layout", "even-vertical"], false, "Set the even-vertical layout"),
    ("M-3", &["select-layout", "main-horizontal"], false, "Set the main-horizontal layout"),
    ("M-4", &["select-layout", "main-vertical"], false, "Set the main-vertical layout"),
    ("M-5", &["select-layout", "tiled"], false, "Select the tiled layout"),
    ("M-6", &["select-layout", "main-horizontal-mirrored"], false, "Set the main-horizontal-mirrored layout"),
    ("M-7", &["select-layout", "main-vertical-mirrored"], false, "Set the main-vertical-mirrored layout"),
    // Copy mode's keys.
    ("[", &["copy-mode"], false, "Enter copy mode"),
    ("PPage", &["copy-mode", "-u"], false, "Enter copy mode and scroll up"),
];

/// The key menu section of a grofe default binding, from its table and
/// first command.
pub(super) fn default_group(table: &str, cmds: &[Vec<String>]) -> &'static str {
    if table == COPY_MODE_VI_TABLE || table == COPY_MODE_TABLE {
        return "copy";
    }
    if table == ROOT_TABLE {
        return "pane";
    }
    let argv = cmds.first().map(Vec::as_slice).unwrap_or_default();
    let word = |i: usize| argv.get(i).map(String::as_str).unwrap_or("");
    let last = argv.last().map(String::as_str).unwrap_or("");
    match word(0) {
        "split-window" | "select-pane" | "resize-pane" | "last-pane" | "rotate-window"
        | "display-panes" | "break-pane" | "swap-pane" | "if-shell" | "clock-mode" | "new-pane" => {
            "pane"
        }
        // `-x W` (`<`) is the window's menu, `-x P` (`>`) the pane's.
        "display-menu" if word(4) == "W" => "window",
        "display-menu" => "pane",
        "new-window" | "next-window" | "previous-window" | "last-window" | "select-window"
        | "kill-window" => "window",
        "switch-client" => "session",
        "new-scratchpad" | "toggle-scratchpad" | "choose-scratchpad" => "scratchpads",
        "settings" => "settings",
        "key-menu" | "list-keys" => "help",
        "confirm-before" if last == "kill-server" => "server",
        "display-message" => "window",
        "choose-tree" if word(1).ends_with('s') => "session",
        "choose-tree" => "window",
        "confirm-before" if last == "kill-pane" => "pane",
        "confirm-before" if last == "kill-window" => "window",
        "command-prompt" if last.starts_with("rename-session") => "session",
        "command-prompt" if last.starts_with("select-pane") => "pane",
        "command-prompt"
            if last.starts_with("rename-window")
                || last.starts_with("move-window")
                || last.starts_with("select-window") =>
        {
            "window"
        }
        "command-prompt" if last.starts_with("list-keys") => "help",
        "command-prompt" => "prompt",
        "copy-mode" | "paste-buffer" | "list-buffers" | "choose-buffer" | "delete-buffer" => "copy",
        "next-layout" | "select-layout" => "layout",
        "agent" | "choose-agent" | "agent-sidebar" | "agent-grid" => "agents",
        "show-messages" | "choose-notification" => "notifications",
        "detach-client" | "choose-client" | "suspend-client" | "refresh-client" => "client",
        _ => "other",
    }
}

/// The sections of the keys reference, in the order it lists them; a group
/// not named here comes after them, in name order.
const GROUP_ORDER: &[&str] = &[
    "window",
    "pane",
    "session",
    "scratchpads",
    "layout",
    "agents",
    "notifications",
    "client",
    "copy",
    "help",
    "prompt",
    "settings",
    "server",
    "other",
];

/// Sorts groups as the keys reference and the key menu list them: by
/// [`GROUP_ORDER`], any other group after those in name order, `other` last.
pub fn sort_groups(groups: &mut [String]) {
    let rank = |g: &str| GROUP_ORDER.iter().position(|o| *o == g);
    groups.sort_by_cached_key(|g| {
        (
            g == "other",
            rank(g).unwrap_or(GROUP_ORDER.len()),
            g.clone(),
        )
    });
}

/// The book's keys reference (`docs/book/src/keys.md`), generated from
/// [`default_bindings`] so it cannot drift: every `prefix` binding of grofe
/// identity by group, with its note and command, and how tmux's own key
/// compares. `tests/docs_reference.rs` compares it with the committed file.
pub fn keys_reference_markdown() -> String {
    fn cell(text: &str) -> String {
        text.replace('|', "\\|")
    }
    let t = default_bindings();
    let mut groups: BTreeMap<String, Vec<(Key, &Binding)>> = BTreeMap::new();
    for key in t.keys(PREFIX_TABLE) {
        if let Some(b) = t.exact(PREFIX_TABLE, &key) {
            let group = b.group.clone().unwrap_or_else(|| "other".to_string());
            groups.entry(group).or_default().push((key, b));
        }
    }
    let mut order: Vec<String> = groups.keys().cloned().collect();
    sort_groups(&mut order);
    let mut out = String::from(
        "# Keys

<!-- Generated from the default bindings in src/server/input/defaults.rs.
     Do not edit by hand: run `just docs` to regenerate. -->

Every default binding of the prefix table, as grofe binds it, by the
section the [key menu](agents.md#the-key-menu) files it under. Press the
prefix key (`Ctrl-b`) first. The last column says how tmux's own default for
that key compares: the same command, grofe's own key, or what tmux binds
there instead. Under tmux identity (grofe run as `tmux`, or
`compat-identity = \"tmux\"`) the keys are tmux's, with tmux's notes.

In the key menu (`prefix Space`), press a listed key to run it, or `C-f` to
search every binding by key, description, group and command. `/` searches
too in a table where `/` is not bound; in the prefix table `/` is
`Describe key binding`. `Enter` runs the selected row, `Escape` leaves search
and then closes the menu.
",
    );
    for group in order {
        out.push_str(&format!(
            "\n## {group}\n\n| Key | Does | Command | tmux |\n|---|---|---|---|\n"
        ));
        for (key, b) in &groups[&group] {
            let command = b
                .cmds
                .iter()
                .map(|argv| {
                    let words: Vec<String> = argv
                        .iter()
                        .map(|a| crate::util::quote::tmux_arg(a))
                        .collect();
                    words.join(" ")
                })
                .collect::<Vec<_>>()
                .join(" ; ");
            let verb = b.cmds.first().and_then(|a| a.first());
            let stock = TMUX_STOCK
                .iter()
                .find(|(name, ..)| parse_key(name).is_ok_and(|k| k == *key));
            let tmux = match stock {
                None => "grofe's own".to_string(),
                Some((_, _, _, text)) if text.split(' ').next() == verb.map(String::as_str) => {
                    "same".to_string()
                }
                Some((_, _, note, _)) => format!("tmux: {note}"),
            };
            out.push_str(&format!(
                "| `{}` | {} | `{}` | {} |\n",
                cell(&key.to_string()),
                cell(b.desc.as_deref().unwrap_or("")),
                cell(&command),
                cell(&tmux),
            ));
        }
    }
    out
}

/// grofe identity's `MouseDown1Control7`, the tab's float control: the
/// pane clicked floats, or unfloats if it is floating.
pub(crate) const GROFE_FLOAT_CONTROL: &str =
    r##"if-shell -F -t = "#{pane_floating_flag}" { unfloat-pane -t = } { float-pane -t = }"##;

/// grofe identity's `MouseDown1Status`: the agents badge opens the
/// notification tray, any other range is tmux's `switch-client -t =`.
pub(crate) const GROFE_STATUS_CLICK: &str = r##"if-shell -F "#{==:#{mouse_status_range},agents}" { choose-notification } { switch-client -t = }"##;

/// grofe identity's `MouseDown3Pane` (FR.4): a pane that asked for the
/// mouse is focused and gets the click, as tmux's stock row sends it on;
/// any other opens
/// the pane's menu at the pointer, its items acting on the pane clicked.
/// Float shows on a tiled pane and Unfloat on a floating one.
pub(crate) fn grofe_pane_menu() -> String {
    format!(
        r##"if-shell -F "#{{mouse_any_flag}}" {{ select-pane -t = ; send-keys -M }} {{ {GROFE_PANE_MENU_TITLE} {} }}"##,
        grofe_pane_menu_items(Some("="))
    )
}

/// The title of grofe identity's pane menu, on a right click and on
/// `prefix >`.
const GROFE_PANE_MENU_TITLE: &str =
    r##"display-menu -T "#[align=centre]#{pane_index} (#{pane_id})""##;

/// The items of grofe identity's pane menu, acting on `target` (`=`, the
/// pane clicked), or on the active pane with none (`prefix >`). Rename
/// selects the target and prompts with its title: `##` keeps the format
/// for the prompt, as a menu expands its rows when it opens, no `%N` pane
/// id goes into the template, where `%1` is the answer, and `%%%` keeps
/// the answer as typed.
pub(crate) fn grofe_pane_menu_items(target: Option<&str>) -> String {
    let t = target.map(|t| format!(" -t {t}")).unwrap_or_default();
    let select = target
        .map(|t| format!("select-pane -t {t} ; "))
        .unwrap_or_default();
    [
        format!(r##""#{{?pane_floating_flag,,Float}}" f {{ float-pane{t} }}"##),
        format!(r##""#{{?pane_floating_flag,Unfloat,}}" f {{ unfloat-pane{t} }}"##),
        "''".to_string(),
        format!(r##""#{{?pane_floating_flag,,Horizontal Split}}" h {{ split-window -h{t} -c "#{{pane_current_path}}" }}"##),
        format!(r##""#{{?pane_floating_flag,,Vertical Split}}" v {{ split-window -v{t} -c "#{{pane_current_path}}" }}"##),
        "''".to_string(),
        format!(r###"Rename n {{ {select}command-prompt -I "##{{pane_title}}" "select-pane -T \"%%%\"" }}"###),
        format!("Kill X {{ kill-pane{t} }}"),
        format!(r##""#{{?#{{>:#{{window_panes}},1}},,-}}#{{?window_zoomed_flag,Unzoom,Zoom}}" z {{ resize-pane -Z{t} }}"##),
    ]
    .join(" ")
}

/// The copy-mode key table with `mode-keys vi`.
pub(crate) const COPY_MODE_VI_TABLE: &str = "copy-mode-vi";

/// tmux 3.7b's 88 stock `copy-mode-vi` bindings (`key-bindings.c`), in
/// native spelling: the key, then its commands as a command string. The
/// rows yank and the navigator add come from those plugins (TM.0's
/// decision 5).
#[rustfmt::skip]
pub(crate) const COPY_MODE_VI: &[(&str, &str)] = &[
    ("#", "send-keys -F -X search-backward -- '#{copy_cursor_word}'"),
    ("*", "send-keys -F -X search-forward -- '#{copy_cursor_word}'"),
    ("C-c", "send-keys -X cancel"),
    ("C-d", "send-keys -X halfpage-down"),
    ("C-e", "send-keys -X scroll-down"),
    ("C-b", "send-keys -X page-up"),
    ("C-f", "send-keys -X page-down"),
    ("C-h", "send-keys -X cursor-left"),
    ("C-j", "send-keys -X copy-pipe-and-cancel"),
    ("Enter", "send-keys -X copy-pipe-and-cancel"),
    ("C-u", "send-keys -X halfpage-up"),
    ("C-v", "send-keys -X rectangle-toggle"),
    ("C-y", "send-keys -X scroll-up"),
    ("Escape", "send-keys -X clear-selection"),
    ("C-[", "send-keys -X clear-selection"),
    ("Space", "send-keys -X begin-selection"),
    ("$", "send-keys -X end-of-line"),
    (",", "send-keys -X jump-reverse"),
    ("/", "command-prompt -T search -p '(search down)' { send-keys -X search-forward -- '%%' }"),
    ("0", "send-keys -X start-of-line"),
    ("1", "command-prompt -N -p '(repeat)' -I 1 { send-keys -N '%%' }"),
    ("2", "command-prompt -N -p '(repeat)' -I 2 { send-keys -N '%%' }"),
    ("3", "command-prompt -N -p '(repeat)' -I 3 { send-keys -N '%%' }"),
    ("4", "command-prompt -N -p '(repeat)' -I 4 { send-keys -N '%%' }"),
    ("5", "command-prompt -N -p '(repeat)' -I 5 { send-keys -N '%%' }"),
    ("6", "command-prompt -N -p '(repeat)' -I 6 { send-keys -N '%%' }"),
    ("7", "command-prompt -N -p '(repeat)' -I 7 { send-keys -N '%%' }"),
    ("8", "command-prompt -N -p '(repeat)' -I 8 { send-keys -N '%%' }"),
    ("9", "command-prompt -N -p '(repeat)' -I 9 { send-keys -N '%%' }"),
    (":", "command-prompt -p '(goto line)' { send-keys -X goto-line -- '%%' }"),
    (";", "send-keys -X jump-again"),
    ("?", "command-prompt -T search -p '(search up)' { send-keys -X search-backward -- '%%' }"),
    ("A", "send-keys -X append-selection-and-cancel"),
    ("B", "send-keys -X previous-space"),
    ("D", "send-keys -X copy-pipe-end-of-line-and-cancel"),
    ("E", "send-keys -X next-space-end"),
    ("F", "command-prompt -1 -p '(jump backward)' { send-keys -X jump-backward -- '%%' }"),
    ("G", "send-keys -X history-bottom"),
    ("H", "send-keys -X top-line"),
    ("J", "send-keys -X scroll-down"),
    ("K", "send-keys -X scroll-up"),
    ("L", "send-keys -X bottom-line"),
    ("M", "send-keys -X middle-line"),
    ("N", "send-keys -X search-reverse"),
    ("P", "send-keys -X toggle-position"),
    ("T", "command-prompt -1 -p '(jump to backward)' { send-keys -X jump-to-backward -- '%%' }"),
    ("V", "send-keys -X select-line"),
    ("W", "send-keys -X next-space"),
    ("X", "send-keys -X set-mark"),
    ("^", "send-keys -X back-to-indentation"),
    ("b", "send-keys -X previous-word"),
    ("e", "send-keys -X next-word-end"),
    ("f", "command-prompt -1 -p '(jump forward)' { send-keys -X jump-forward -- '%%' }"),
    ("g", "send-keys -X history-top"),
    ("h", "send-keys -X cursor-left"),
    ("j", "send-keys -X cursor-down"),
    ("k", "send-keys -X cursor-up"),
    ("z", "send-keys -X scroll-middle"),
    ("l", "send-keys -X cursor-right"),
    ("n", "send-keys -X search-again"),
    ("o", "send-keys -X other-end"),
    ("q", "send-keys -X cancel"),
    ("r", "send-keys -X refresh-from-pane"),
    ("t", "command-prompt -1 -p '(jump to forward)' { send-keys -X jump-to-forward -- '%%' }"),
    ("v", "send-keys -X rectangle-toggle"),
    ("w", "send-keys -X next-word"),
    ("{", "send-keys -X previous-paragraph"),
    ("}", "send-keys -X next-paragraph"),
    ("%", "send-keys -X next-matching-bracket"),
    ("Home", "send-keys -X start-of-line"),
    ("End", "send-keys -X end-of-line"),
    ("MouseDown1Pane", "select-pane"),
    ("MouseDrag1Pane", "select-pane; send-keys -X begin-selection"),
    ("MouseDragEnd1Pane", "send-keys -X copy-pipe-and-cancel"),
    ("WheelUpPane", "select-pane; send-keys -N 5 -X scroll-up"),
    ("WheelDownPane", "select-pane; send-keys -N 5 -X scroll-down"),
    ("DoubleClick1Pane", "select-pane; send-keys -X select-word; run-shell -d 0.3; send-keys -X copy-pipe-and-cancel"),
    ("TripleClick1Pane", "select-pane; send-keys -X select-line; run-shell -d 0.3; send-keys -X copy-pipe-and-cancel"),
    ("BSpace", "send-keys -X cursor-left"),
    ("NPage", "send-keys -X page-down"),
    ("PPage", "send-keys -X page-up"),
    ("Up", "send-keys -X cursor-up"),
    ("Down", "send-keys -X cursor-down"),
    ("Left", "send-keys -X cursor-left"),
    ("Right", "send-keys -X cursor-right"),
    ("M-x", "send-keys -X jump-to-mark"),
    ("C-Up", "send-keys -X scroll-up"),
    ("C-Down", "send-keys -X scroll-down"),
];

/// The copy-mode key table with `mode-keys emacs`.
pub(crate) const COPY_MODE_TABLE: &str = "copy-mode";

/// tmux 3.7b's 75 stock `copy-mode` (emacs) bindings (`key-bindings.c`), in
/// native spelling like [`COPY_MODE_VI`].
#[rustfmt::skip]
pub(crate) const COPY_MODE_EMACS: &[(&str, &str)] = &[
    ("Escape", "send-keys -X cancel"),
    ("Space", "send-keys -X page-down"),
    (",", "send-keys -X jump-reverse"),
    (";", "send-keys -X jump-again"),
    ("F", "command-prompt -1 -p '(jump backward)' { send-keys -X jump-backward -- '%%' }"),
    ("N", "send-keys -X search-reverse"),
    ("P", "send-keys -X toggle-position"),
    ("R", "send-keys -X rectangle-toggle"),
    ("T", "command-prompt -1 -p '(jump to backward)' { send-keys -X jump-to-backward -- '%%' }"),
    ("X", "send-keys -X set-mark"),
    ("f", "command-prompt -1 -p '(jump forward)' { send-keys -X jump-forward -- '%%' }"),
    ("g", "command-prompt -p '(goto line)' { send-keys -X goto-line -- '%%' }"),
    ("n", "send-keys -X search-again"),
    ("q", "send-keys -X cancel"),
    ("r", "send-keys -X refresh-from-pane"),
    ("t", "command-prompt -1 -p '(jump to forward)' { send-keys -X jump-to-forward -- '%%' }"),
    ("Home", "send-keys -X start-of-line"),
    ("End", "send-keys -X end-of-line"),
    ("NPage", "send-keys -X page-down"),
    ("PPage", "send-keys -X page-up"),
    ("Up", "send-keys -X cursor-up"),
    ("Down", "send-keys -X cursor-down"),
    ("Left", "send-keys -X cursor-left"),
    ("Right", "send-keys -X cursor-right"),
    ("MouseDown1Pane", "select-pane"),
    ("MouseDrag1Pane", "select-pane; send-keys -X begin-selection"),
    ("MouseDragEnd1Pane", "send-keys -X copy-pipe-and-cancel"),
    ("WheelDownPane", "select-pane; send-keys -N 5 -X scroll-down"),
    ("WheelUpPane", "select-pane; send-keys -N 5 -X scroll-up"),
    ("DoubleClick1Pane", "select-pane; send-keys -X select-word; run-shell -d 0.3; send-keys -X copy-pipe-and-cancel"),
    ("TripleClick1Pane", "select-pane; send-keys -X select-line; run-shell -d 0.3; send-keys -X copy-pipe-and-cancel"),
    ("M-1", "command-prompt -N -p '(repeat)' -I 1 { send-keys -N '%%' }"),
    ("M-2", "command-prompt -N -p '(repeat)' -I 2 { send-keys -N '%%' }"),
    ("M-3", "command-prompt -N -p '(repeat)' -I 3 { send-keys -N '%%' }"),
    ("M-4", "command-prompt -N -p '(repeat)' -I 4 { send-keys -N '%%' }"),
    ("M-5", "command-prompt -N -p '(repeat)' -I 5 { send-keys -N '%%' }"),
    ("M-6", "command-prompt -N -p '(repeat)' -I 6 { send-keys -N '%%' }"),
    ("M-7", "command-prompt -N -p '(repeat)' -I 7 { send-keys -N '%%' }"),
    ("M-8", "command-prompt -N -p '(repeat)' -I 8 { send-keys -N '%%' }"),
    ("M-9", "command-prompt -N -p '(repeat)' -I 9 { send-keys -N '%%' }"),
    ("M-<", "send-keys -X history-top"),
    ("M->", "send-keys -X history-bottom"),
    ("M-R", "send-keys -X top-line"),
    ("M-b", "send-keys -X previous-word"),
    ("M-f", "send-keys -X next-word-end"),
    ("M-l", "send-keys -X cursor-centre-horizontal"),
    ("M-m", "send-keys -X back-to-indentation"),
    ("M-r", "send-keys -X middle-line"),
    ("M-v", "send-keys -X page-up"),
    ("M-w", "send-keys -X copy-pipe-and-cancel"),
    ("M-x", "send-keys -X jump-to-mark"),
    ("M-{", "send-keys -X previous-paragraph"),
    ("M-}", "send-keys -X next-paragraph"),
    ("M-Up", "send-keys -X halfpage-up"),
    ("M-Down", "send-keys -X halfpage-down"),
    ("C-Space", "send-keys -X begin-selection"),
    ("C-[", "send-keys -X cancel"),
    ("C-a", "send-keys -X start-of-line"),
    ("C-b", "send-keys -X cursor-left"),
    ("C-c", "send-keys -X cancel"),
    ("C-e", "send-keys -X end-of-line"),
    ("C-f", "send-keys -X cursor-right"),
    ("C-g", "send-keys -X clear-selection"),
    ("C-k", "send-keys -X copy-pipe-end-of-line-and-cancel"),
    ("C-l", "send-keys -X recentre-top-bottom"),
    ("C-n", "send-keys -X cursor-down"),
    ("C-p", "send-keys -X cursor-up"),
    ("C-r", "command-prompt -T search -i -I '#{pane_search_string}' -p '(search up)' { send-keys -X search-backward-incremental -- '%%' }"),
    ("C-s", "command-prompt -T search -i -I '#{pane_search_string}' -p '(search down)' { send-keys -X search-forward-incremental -- '%%' }"),
    ("C-v", "send-keys -X page-down"),
    ("C-w", "send-keys -X copy-pipe-and-cancel"),
    ("C-Up", "send-keys -X scroll-up"),
    ("C-Down", "send-keys -X scroll-down"),
    ("C-M-b", "send-keys -X previous-matching-bracket"),
    ("C-M-f", "send-keys -X next-matching-bracket"),
];
