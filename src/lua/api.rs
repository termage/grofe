use super::callbacks::{CALLBACKS, LINES, NEXT_ID, add_callback, free_line};
pub use super::callbacks::{Payload, call, in_pack, release};
use super::pack;
use super::runtime::{LuaError, Runtime};
use super::system;
pub use super::system::SystemJob;
use mlua::{Function, Lua, Table, Value};
use std::cell::RefCell;

pub trait Host {
    fn run(&mut self, argv: Vec<String>) -> Result<String, String>;
    fn expand(&mut self, format: &str, target: Option<&str>) -> Result<String, String>;
    fn bind(
        &mut self,
        table: &str,
        key: &str,
        id: u32,
        desc: Option<String>,
        repeat: bool,
    ) -> Result<(), String>;
    /// Calls `id` for `event`, only for `pane` when it is given.
    fn subscribe(&mut self, event: &str, id: u32, pane: Option<&str>) -> Result<(), String>;
    fn every(&mut self, ms: u64, id: u32) -> Result<(), String>;
    fn system(&mut self, job: SystemJob, id: u32) -> Result<(), String>;
    /// The last `n` messages of `pane`'s agent, oldest first.
    fn agent_history(&mut self, pane: &str, n: usize) -> Result<Vec<String>, String>;
    fn cancel(&mut self, id: u32);
    /// Posts a tray entry for `pane` (the active pane when `None`).
    fn notify(&mut self, text: &str, pane: Option<&str>, level: &str) -> Result<(), String>;
    fn log(&mut self, msg: &str);
}

const SLOT: &str = "grofe_host";
const PLUGINS: &str = "grofe_plugins";
const MIN_EVERY_MS: u64 = 100;
const LEVELS: [&str; 3] = ["info", "warn", "error"];

fn err(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(e.to_string())
}

fn setup(e: mlua::Error) -> LuaError {
    LuaError {
        who: "grofe".into(),
        message: e.to_string(),
    }
}

/// The scoped functions `enter` put in place, or an error naming `name`.
fn slot(lua: &Lua, name: &str) -> mlua::Result<Table> {
    match lua.named_registry_value::<Value>(SLOT)? {
        Value::Table(t) => Ok(t),
        _ => Err(err(format!("grofe.{name} outside a grofe call"))),
    }
}

pub(super) fn host_fn(lua: &Lua, name: &str) -> mlua::Result<Function> {
    slot(lua, name)?.get(name)
}

fn strings(t: Table) -> mlua::Result<Vec<String>> {
    t.sequence_values::<String>().collect()
}

pub fn install(rt: &Runtime) -> Result<(), LuaError> {
    install_in(rt.lua()).map_err(setup)
}

fn install_in(lua: &Lua) -> mlua::Result<()> {
    lua.set_named_registry_value(CALLBACKS, lua.create_table()?)?;
    lua.set_named_registry_value(PLUGINS, lua.create_table()?)?;
    lua.set_named_registry_value(NEXT_ID, 1u32)?;
    lua.set_named_registry_value(LINES, lua.create_table()?)?;
    let g = lua.create_table()?;
    g.set("api", crate::pack::PLUGIN_API)?;
    g.set(
        "cmd",
        lua.create_function(|lua, arg: Value| {
            let run = host_fn(lua, "cmd")?;
            let commands = match arg {
                Value::String(s) => {
                    crate::cmd::parse::split_command_string(&s.to_str()?).map_err(err)?
                }
                Value::Table(t) => vec![strings(t)?],
                _ => return Err(err("grofe.cmd takes an argv table or a string")),
            };
            let mut out = Vec::new();
            for argv in commands {
                out.push(run.call::<String>(argv)?);
            }
            Ok(out.join("\n"))
        })?,
    )?;
    g.set(
        "fmt",
        lua.create_function(|lua, (format, target): (String, Option<String>)| {
            host_fn(lua, "fmt")?.call::<String>((format, target))
        })?,
    )?;
    g.set(
        "bind",
        lua.create_function(
            |lua, (table, key, f, opts): (String, String, Function, Option<Table>)| {
                let host = host_fn(lua, "bind")?;
                let (desc, repeat) = match opts {
                    Some(o) => (
                        o.get::<Option<String>>("desc")?,
                        o.get::<Option<bool>>("repeat")?.unwrap_or(false),
                    ),
                    None => (None, false),
                };
                add_callback(lua, f, |id| host.call((table, key, id, desc, repeat)))
            },
        )?,
    )?;
    g.set(
        "on",
        lua.create_function(|lua, (event, f, opts): (String, Function, Option<Table>)| {
            let host = host_fn(lua, "on")?;
            let pane = match opts {
                Some(o) => {
                    for pair in o.pairs::<Value, Value>() {
                        match pair?.0 {
                            Value::String(s) if &*s.to_str()? == "pane" => {}
                            other => {
                                return Err(err(format!(
                                    "grofe.on: unknown option {}; options are pane",
                                    other.to_string()?
                                )));
                            }
                        }
                    }
                    o.get::<Option<String>>("pane")?
                }
                None => None,
            };
            add_callback(lua, f, |id| host.call((event, id, pane)))
        })?,
    )?;
    g.set(
        "every",
        lua.create_function(|lua, (ms, f): (f64, Function)| {
            let host = host_fn(lua, "every")?;
            let ms = (ms.max(0.0) as u64).max(MIN_EVERY_MS);
            add_callback(lua, f, |id| host.call((ms, id)))
        })?,
    )?;
    g.set(
        "system",
        lua.create_function(|lua, (argv, f, opts): (Table, Function, Option<Table>)| {
            let host = host_fn(lua, "system")?;
            let (mut job, on_line) = system::read(strings(argv)?, opts)?;
            let line_id = on_line
                .map(|line| add_callback(lua, line, |_| Ok(())))
                .transpose()?;
            job.on_line = line_id;
            let id = add_callback(lua, f, |id| host.call((job.to_table(lua)?, id)));
            match (&id, line_id) {
                (Ok(id), Some(line)) => lua.named_registry_value::<Table>(LINES)?.set(*id, line)?,
                (Err(_), Some(line)) => lua
                    .named_registry_value::<Table>(CALLBACKS)?
                    .set(line, Value::Nil)?,
                _ => {}
            }
            id
        })?,
    )?;
    let agent = lua.create_table()?;
    agent.set(
        "history",
        lua.create_function(|lua, (pane, n): (String, f64)| {
            let n = n.max(0.0) as usize;
            host_fn(lua, "agent_history")?.call::<Vec<String>>((pane, n))
        })?,
    )?;
    g.set("agent", agent)?;
    g.set(
        "cancel",
        lua.create_function(|lua, id: u32| {
            host_fn(lua, "cancel")?.call::<()>(id)?;
            free_line(lua, id)?;
            lua.named_registry_value::<Table>(CALLBACKS)?
                .set(id, Value::Nil)
        })?,
    )?;
    g.set(
        "log",
        lua.create_function(|lua, msg: String| host_fn(lua, "log")?.call::<()>(msg))?,
    )?;
    g.set(
        "notify",
        lua.create_function(|lua, (text, opts): (Value, Option<Table>)| {
            let Value::String(text) = text else {
                return Err(err("grofe.notify takes the text as a string"));
            };
            let (pane, level) = match opts {
                Some(o) => (
                    o.get::<Option<String>>("pane")?,
                    o.get::<Option<String>>("level")?,
                ),
                None => (None, None),
            };
            let level = level.unwrap_or_else(|| "info".into());
            if !LEVELS.contains(&level.as_str()) {
                return Err(err(format!(
                    "grofe.notify: level must be one of {LEVELS:?}"
                )));
            }
            host_fn(lua, "notify")?.call::<()>((text.to_str()?.to_string(), pane, level))
        })?,
    )?;
    g.set(
        "plugin",
        lua.create_function(|lua, name: String| {
            lua.named_registry_value::<Table>(PLUGINS)?
                .get::<Value>(name)
        })?,
    )?;
    super::json::install(lua, &g)?;
    super::config::install(lua, &g)?;
    pack::install(lua, &g)?;
    lua.globals().set("grofe", g)
}

pub fn enter<R>(
    rt: &Runtime,
    host: &mut dyn Host,
    who: &str,
    f: impl FnOnce(&Lua) -> mlua::Result<R>,
) -> Result<R, LuaError> {
    rt.guarded(who, |lua| {
        let host = RefCell::new(host);
        let result = lua.scope(|scope| {
            let t = lua.create_table()?;
            t.set(
                "cmd",
                scope.create_function_mut(|_, argv: Vec<String>| {
                    host.borrow_mut().run(argv).map_err(err)
                })?,
            )?;
            t.set(
                "fmt",
                scope.create_function_mut(|_, (format, target): (String, Option<String>)| {
                    host.borrow_mut()
                        .expand(&format, target.as_deref())
                        .map_err(err)
                })?,
            )?;
            t.set(
                "bind",
                scope.create_function_mut(
                    |_,
                     (table, key, id, desc, repeat): (
                        String,
                        String,
                        u32,
                        Option<String>,
                        bool,
                    )| {
                        host.borrow_mut()
                            .bind(&table, &key, id, desc, repeat)
                            .map_err(err)
                    },
                )?,
            )?;
            t.set(
                "on",
                scope.create_function_mut(
                    |_, (event, id, pane): (String, u32, Option<String>)| {
                        host.borrow_mut()
                            .subscribe(&event, id, pane.as_deref())
                            .map_err(err)
                    },
                )?,
            )?;
            t.set(
                "every",
                scope.create_function_mut(|_, (ms, id): (u64, u32)| {
                    host.borrow_mut().every(ms, id).map_err(err)
                })?,
            )?;
            t.set(
                "system",
                scope.create_function_mut(|_, (job, id): (Table, u32)| {
                    let job = SystemJob::from_table(&job)?;
                    host.borrow_mut().system(job, id).map_err(err)
                })?,
            )?;
            t.set(
                "agent_history",
                scope.create_function_mut(|_, (pane, n): (String, usize)| {
                    host.borrow_mut().agent_history(&pane, n).map_err(err)
                })?,
            )?;
            t.set(
                "cancel",
                scope.create_function_mut(|_, id: u32| {
                    host.borrow_mut().cancel(id);
                    Ok(())
                })?,
            )?;
            t.set(
                "notify",
                scope.create_function_mut(
                    |_, (text, pane, level): (String, Option<String>, String)| {
                        host.borrow_mut()
                            .notify(&text, pane.as_deref(), &level)
                            .map_err(err)
                    },
                )?,
            )?;
            t.set(
                "log",
                scope.create_function_mut(|_, msg: String| {
                    host.borrow_mut().log(&msg);
                    Ok(())
                })?,
            )?;
            lua.set_named_registry_value(SLOT, t)?;
            let r = f(lua);
            lua.set_named_registry_value(SLOT, Value::Nil)?;
            r
        });
        // A failed scope setup must not leave a dead slot behind.
        let _ = lua.set_named_registry_value(SLOT, Value::Nil);
        result
    })
}

pub fn set_plugin(rt: &Runtime, name: &str, value: Value) -> Result<(), LuaError> {
    rt.lua()
        .named_registry_value::<Table>(PLUGINS)
        .and_then(|t| t.set(name, value))
        .map_err(setup)
}

#[cfg(test)]
mod tests;
