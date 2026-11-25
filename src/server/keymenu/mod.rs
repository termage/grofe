//! The key menu (agents spec §5.5): a key table's bindings in the list
//! widget, grouped. A key bound in the table runs at once; `C-n`, `C-p`,
//! `C-f`, `Escape`, `C-c` and the unmodified `Up`, `Down`, `PageUp`,
//! `PageDown`, `Home` and `End` belong to the menu.

use std::collections::BTreeMap;
use std::time::Instant;

mod draw;
#[cfg(test)]
mod tests;

pub use draw::KeyMenuOverlay;
use draw::{list_height, page_starts};

use crate::server::State;
use crate::server::clients;
use crate::server::input::{
    Input, InputHook, Key, KeyCode, KeySettings, KeyState, KeyTables, Mods, Routed, sort_groups,
};
use crate::server::listview::{self, ListAction, ListState};
use crate::server::prompt;
use crate::types::{ClientId, InputModes};

/// Group of a binding with none, listed last.
const OTHER: &str = "other";

/// Open menus, per client. Lives on `State.keymenus`.
#[derive(Default)]
pub struct Menus {
    open: BTreeMap<ClientId, Menu>,
}

impl Menus {
    pub fn is_open(&self, id: ClientId) -> bool {
        self.open.contains_key(&id)
    }
}

struct Menu {
    table: String,
    ui: ListState,
}

/// One listed binding.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    key: Key,
    group: String,
    /// The description, or the command when there is none.
    label: String,
    /// The binding's commands, joined with `; `.
    cmd: String,
    described: bool,
}

impl Row {
    /// What search matches against: key, description, group and command,
    /// in that order so the drawn parts' match positions stay put.
    fn haystack(&self) -> String {
        format!("{} {} {} {}", self.key, self.label, self.group, self.cmd)
    }
}

/// What a key did to the menu.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Run this key's binding and close.
    Run(Key),
    /// Open this table's menu instead.
    Submenu(String),
    Close,
    Redraw,
    Ignore,
}

/// Opens `table`'s menu on `client`.
pub fn open(state: &mut State, client: ClientId, table: &str) -> Result<(), String> {
    if !state.clients.contains_key(&client) {
        return Err("no client".to_string());
    }
    if !state.registry.key_tables.exists(table) {
        return Err(format!("unknown key table: {table}"));
    }
    state.keymenus.open.insert(
        client,
        Menu {
            table: table.to_string(),
            ui: ListState::default(),
        },
    );
    clients::mark_dirty(state, client);
    Ok(())
}

fn close(state: &mut State, client: ClientId) {
    state.keymenus.open.remove(&client);
    clients::mark_dirty(state, client);
}

/// A client that detached takes its menu with it.
pub fn client_gone(state: &mut State, id: ClientId) {
    state.keymenus.open.remove(&id);
}

fn is_reserved(key: &Key) -> bool {
    let ctrl = key.mods.ctrl && !key.mods.alt;
    match key.code {
        KeyCode::Escape => true,
        KeyCode::Char('n' | 'p' | 'f' | 'c') => ctrl,
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::Home
        | KeyCode::End => key.mods == Mods::default(),
        _ => false,
    }
}

/// The table's bindings by group, groups as [`sort_groups`] orders them.
/// Mouse keys and `Any` are left out. `expand` turns a group or
/// description format into text.
fn build_rows(tables: &KeyTables, table: &str, expand: &dyn Fn(&str) -> String) -> Vec<Row> {
    let mut groups: Vec<String> = Vec::new();
    let mut found: Vec<Row> = Vec::new();
    for key in tables.keys(table) {
        if matches!(key.code, KeyCode::Mouse(_) | KeyCode::Any) {
            continue;
        }
        let Some(b) = tables.exact(table, &key) else {
            continue;
        };
        let group = expand(b.group.as_deref().unwrap_or(OTHER));
        if !groups.contains(&group) {
            groups.push(group.clone());
        }
        let cmd = b
            .cmds
            .iter()
            .map(|argv| argv.join(" "))
            .collect::<Vec<_>>()
            .join("; ");
        let label = b.desc.as_deref().map_or_else(|| cmd.clone(), expand);
        found.push(Row {
            key,
            group,
            label,
            described: b.desc.is_some(),
            cmd,
        });
    }
    sort_groups(&mut groups);
    let mut rows = Vec::with_capacity(found.len());
    for g in &groups {
        rows.extend(found.iter().filter(|r| &r.group == g).cloned());
    }
    rows
}

/// Positions in `rows` the query leaves visible, best first.
fn visible(rows: &[Row], query: &str) -> Vec<usize> {
    let texts: Vec<String> = rows.iter().map(Row::haystack).collect();
    listview::filter(&texts, query)
}

/// Whether `/` runs a binding in `table`, so only `C-f` searches.
fn slash_bound(tables: &KeyTables, table: &str) -> bool {
    tables
        .get(table, &Key::new(KeyCode::Char('/'), Mods::default()))
        .is_some()
}

/// A page key in the grouped view moves to the first row of the next or
/// previous page as drawn (`pages` holds those rows). False for any other
/// key, which [`step`] then takes.
fn page_key(ui: &mut ListState, pages: &[usize], last: usize, key: &Key) -> bool {
    let down = key.code == KeyCode::PageDown;
    if !(down || key.code == KeyCode::PageUp) || key.mods != Mods::default() {
        return false;
    }
    if ui.filtering || pages.is_empty() {
        return false;
    }
    let page = pages.iter().rposition(|&p| p <= ui.selected).unwrap_or(0);
    ui.selected = match down {
        true => pages.get(page + 1).copied().unwrap_or(last),
        false => pages[page.saturating_sub(1)],
    };
    true
}

/// What `key` does in the menu of `table`.
fn step(ui: &mut ListState, rows: &[Row], tables: &KeyTables, table: &str, key: &Key) -> Step {
    let plain = key.mods == Mods::default();
    if ui.filtering
        && plain
        && (key.code == KeyCode::Escape || key.code == KeyCode::Backspace && ui.query.is_empty())
    {
        *ui = ListState {
            page: ui.page,
            ..ListState::default()
        };
        return Step::Redraw;
    }
    if !ui.filtering && plain && key.code == KeyCode::Char('/') && !slash_bound(tables, table) {
        ui.filtering = true;
        return Step::Redraw;
    }
    let shown = visible(rows, &ui.query);
    let reserved = is_reserved(key);
    if !ui.filtering
        && !reserved
        && !matches!(key.code, KeyCode::Mouse(_) | KeyCode::Any)
        && let Some(b) = tables.get(table, key)
    {
        if let [argv] = b.cmds.as_slice()
            && let [cmd, flag, target] = argv.as_slice()
            && matches!(cmd.as_str(), "switch-client" | "switchc")
            && flag == "-T"
        {
            return Step::Submenu(target.clone());
        }
        return Step::Run(*key);
    }
    match ui.on_key(key, shown.len(), false) {
        ListAction::Close => Step::Close,
        ListAction::Moved | ListAction::QueryChanged => Step::Redraw,
        ListAction::Enter => match shown.get(ui.selected).and_then(|i| rows.get(*i)) {
            Some(row) => Step::Run(row.key),
            None => Step::Ignore,
        },
        ListAction::Ctrl(_) | ListAction::None => Step::Ignore,
    }
}

/// Presses `key` in `table` for a client as the key tables would: a `-r`
/// binding leaves the client in the table, any other returns it to root.
fn press(
    keys: &mut KeyState,
    tables: &KeyTables,
    table: &str,
    key: &Key,
    settings: &KeySettings,
    modes: &InputModes,
) -> Routed {
    keys.set_table(table);
    keys.route(&Input::Key(*key), Instant::now(), tables, settings, modes)
}

// --- Keys ---------------------------------------------------------------

/// Takes a client's keys while its key menu is open.
pub struct KeyMenuHook;

impl InputHook for KeyMenuHook {
    fn on_input(
        &self,
        state: &mut State,
        client: ClientId,
        input: &Input,
        _now: Instant,
    ) -> Option<Routed> {
        if !state.keymenus.is_open(client) {
            return None;
        }
        let Some(session) = state.clients.get(&client).map(|c| c.session) else {
            state.keymenus.open.remove(&client);
            return None;
        };
        let key = match input {
            Input::Key(key) => *key,
            Input::Paste { .. } | Input::Mouse(_) => return Some(Routed::Consumed),
            Input::Focus(_) => return None,
        };
        let table = state.keymenus.open.get(&client)?.table.clone();
        // The prefix key is not a binding to run from a menu: it would
        // switch the client to the prefix table under the menu.
        let settings = clients::key_settings(&state.model, session);
        if settings.is_prefix(&key) {
            return Some(Routed::Consumed);
        }
        let rows = rows_for(state, client, &table);
        let size = state.clients.get(&client).map_or((80, 24), |c| c.size);
        let outcome = {
            let menu = state.keymenus.open.get_mut(&client)?;
            // A page key moves a screenful: the list's height is known to
            // `ListState` before the key, not only after it.
            let n = visible(&rows, &menu.ui.query).len();
            menu.ui.clamp(n, list_height(size, menu.ui.filtering));
            let pages = page_starts(&rows, size);
            let last = rows.len().saturating_sub(1);
            let outcome = if page_key(&mut menu.ui, &pages, last, &key) {
                Step::Redraw
            } else {
                step(
                    &mut menu.ui,
                    &rows,
                    &state.registry.key_tables,
                    &table,
                    &key,
                )
            };
            let n = visible(&rows, &menu.ui.query).len();
            menu.ui.clamp(n, list_height(size, menu.ui.filtering));
            outcome
        };
        match outcome {
            Step::Ignore => {}
            Step::Redraw => clients::mark_dirty(state, client),
            Step::Close => close(state, client),
            Step::Submenu(target) => {
                if state.registry.key_tables.exists(&target) {
                    if let Some(menu) = state.keymenus.open.get_mut(&client) {
                        menu.table = target;
                        menu.ui = ListState::default();
                    }
                    clients::mark_dirty(state, client);
                } else {
                    close(state, client);
                }
            }
            Step::Run(run) => {
                close(state, client);
                let modes = active_pane(state, session)
                    .and_then(|p| state.model.panes.get(&p))
                    .map(|p| p.handle.modes())
                    .unwrap_or_default();
                let State {
                    clients, registry, ..
                } = &mut *state;
                let c = clients.get_mut(&client)?;
                return Some(press(
                    &mut c.keys,
                    &registry.key_tables,
                    &table,
                    &run,
                    &settings,
                    &modes,
                ));
            }
        }
        Some(Routed::Consumed)
    }
}

fn active_pane(state: &State, session: crate::types::SessionId) -> Option<crate::types::PaneId> {
    let model = &state.model;
    let window = model.sessions.get(&session)?.active;
    Some(model.windows.get(&window)?.active)
}

fn rows_for(state: &State, client: ClientId, table: &str) -> Vec<Row> {
    let Some(c) = state.clients.get(&client) else {
        return Vec::new();
    };
    let mut ctx = c.ctx.clone();
    ctx.pane = active_pane(state, c.session);
    build_rows(&state.registry.key_tables, table, &|text| {
        prompt::expand_command(state, &ctx, text)
    })
}
