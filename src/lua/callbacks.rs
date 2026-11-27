//! Callbacks Luau registered (keys, timers, events, processes): their ids,
//! who owns them, and running them.

use super::api::Host;
use super::api::{enter, host_fn};
use super::pack;
use super::runtime::{LuaError, Runtime};
use mlua::{Function, Lua, Table, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    None,
    Event(serde_json::Value),
    /// One line of a child's output, for `on_line`.
    Line(String),
    System {
        code: i32,
        stdout: String,
        stderr: String,
        /// Killed by its `timeout_ms`.
        timed_out: bool,
        /// Why the command could not be spawned.
        error: Option<String>,
        /// The signal that killed it.
        signal: Option<i32>,
    },
}

pub(super) const CALLBACKS: &str = "grofe_callbacks";
pub(super) const NEXT_ID: &str = "grofe_next_id";
/// Per `system` callback id, the id of its `on_line` callback.
pub(super) const LINES: &str = "grofe_lines";

/// Forgets callback `id`, whose task is gone.
pub fn release(rt: &Runtime, id: u32) {
    let _ = rt
        .lua()
        .named_registry_value::<Table>(CALLBACKS)
        .and_then(|t| t.set(id, Value::Nil));
}

/// Forgets the `on_line` callback of `system` callback `id`, if it has one.
pub(super) fn free_line(lua: &Lua, id: u32) -> mlua::Result<()> {
    let lines = lua.named_registry_value::<Table>(LINES)?;
    if let Some(line) = lines.get::<Option<u32>>(id)? {
        lua.named_registry_value::<Table>(CALLBACKS)?
            .set(line, Value::Nil)?;
        lines.set(id, Value::Nil)?;
    }
    Ok(())
}

/// Stores `f` under the next callback id, after `register` accepted that id.
pub(super) fn add_callback(
    lua: &Lua,
    f: Function,
    register: impl FnOnce(u32) -> mlua::Result<()>,
) -> mlua::Result<u32> {
    let id: u32 = lua.named_registry_value(NEXT_ID)?;
    register(id)?;
    lua.named_registry_value::<Table>(CALLBACKS)?.set(id, f)?;
    pack::claim(lua, id)?;
    lua.set_named_registry_value(NEXT_ID, id + 1)?;
    Ok(id)
}

pub fn call(rt: &Runtime, host: &mut dyn Host, id: u32, payload: Payload) -> Result<(), LuaError> {
    let owner = pack::owner(rt.lua(), id).unwrap_or(None);
    let who = match &owner {
        Some(pack) => format!("pack {pack} callback {id}"),
        None => format!("callback {id}"),
    };
    enter(rt, host, &who, |lua| run_callback(lua, id, payload))
}

fn run_callback(lua: &Lua, id: u32, payload: Payload) -> mlua::Result<()> {
    let f: Value = lua.named_registry_value::<Table>(CALLBACKS)?.get(id)?;
    let Value::Function(f) = f else {
        return Ok(());
    };
    match payload {
        Payload::None => f.call::<()>(()),
        Payload::Line(l) => f.call::<()>(l),
        Payload::Event(v) => f.call::<()>(super::json::from_json(lua, &v)?),
        Payload::System {
            code,
            stdout,
            stderr,
            timed_out,
            error,
            signal,
        } => {
            // Its lines were all delivered before this payload.
            free_line(lua, id)?;
            let t = lua.create_table()?;
            t.set("code", code)?;
            t.set("stdout", stdout)?;
            t.set("stderr", stderr)?;
            t.set("timed_out", timed_out)?;
            t.set("error", error)?;
            t.set("signal", signal)?;
            f.call::<()>(t)
        }
    }
}

/// Runs `f` (a pack's chunk): if it fails, everything registered since
/// (keys, timers, subscriptions) goes. Call inside `enter`.
pub fn in_pack<R>(lua: &Lua, f: impl FnOnce() -> mlua::Result<R>) -> mlua::Result<R> {
    let from: u32 = lua.named_registry_value(NEXT_ID)?;
    let result = f();
    if result.is_err() {
        unregister_from(lua, from);
    }
    result
}

/// Cancels and forgets every callback made since id `from`; what cannot be
/// undone is logged.
fn unregister_from(lua: &Lua, from: u32) {
    let until: u32 = lua.named_registry_value(NEXT_ID).unwrap_or(from);
    let mut problems = Vec::new();
    // Newest first, so a key bound twice gets its first owner back.
    for id in (from..until).rev() {
        let undone = host_fn(lua, "cancel")
            .and_then(|cancel| cancel.call::<()>(id))
            .and_then(|()| free_line(lua, id))
            .and_then(|()| {
                lua.named_registry_value::<Table>(CALLBACKS)?
                    .set(id, Value::Nil)
            });
        if let Err(e) = undone {
            problems.push(format!("callback {id}: {e}"));
        }
    }
    if !problems.is_empty()
        && let Ok(log) = host_fn(lua, "log")
    {
        let _ = log.call::<()>(format!("could not unregister {}", problems.join("; ")));
    }
}
