//! The `Host` for the server's `State`: Luau's entry points on the actor.
//! `state.lua` is taken out of `State` for every call and put back after,
//! so the host can hold `&mut State`; a call that finds it taken is nested.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cmd;
use crate::lua::api::{self, Host, Payload};
use crate::lua::runtime::{Limits, LuaError, Runtime};
use crate::lua::{pack, require};
use crate::protocol::{ClientCtx, Dialect};
use crate::server::input::{Binding, BindingSource, Key, parse_key};
use crate::server::lua_async::Async;
use crate::server::{State, clients, dispatch, prompt};

const PLUGIN: &str = "lua";

pub struct LuaHost {
    rt: Runtime,
    async_: Async,
    /// Per bind id, the key it took and what held that key before.
    replaced: HashMap<u32, Replaced>,
}

struct Replaced {
    table: String,
    key: Key,
    previous: Option<Binding>,
}

thread_local! {
    /// Set while a Luau call runs on this thread (the actor's).
    static IN_CALL: Cell<bool> = const { Cell::new(false) };
    /// A reload that reached us inside a call: what it asked to load, run
    /// as a job once the call has ended.
    static DEFERRED: RefCell<Option<Deferred>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct Deferred {
    packs: Vec<(String, PathBuf, serde_json::Value)>,
}

/// Whether a reload is being collected for after the current call.
fn deferring() -> bool {
    IN_CALL.with(Cell::get) && DEFERRED.with(|d| d.borrow().is_some())
}

/// The reload sequence over what a nested reload collected.
fn run_deferred(state: &mut State, d: Deferred, config_dir: PathBuf) {
    reset_at(state, &config_dir);
    for (name, dir, options) in &d.packs {
        if let Err(e) = load_pack(state, name, dir, options) {
            log(state, format!("lua: {e}"));
        }
    }
    load_init(state, &config_dir);
}

struct InCall;

impl InCall {
    fn enter() -> Option<InCall> {
        IN_CALL.with(|c| if c.replace(true) { None } else { Some(InCall) })
    }
}

impl Drop for InCall {
    fn drop(&mut self) {
        IN_CALL.with(|c| c.set(false));
    }
}

struct StateHost<'a> {
    state: &'a mut State,
    async_: &'a mut Async,
    replaced: &'a mut HashMap<u32, Replaced>,
    ctx: ClientCtx,
}

/// What a key bound by `grofe.bind` runs for callback `id`.
fn lua_call(id: u32) -> Vec<Vec<String>> {
    vec![vec!["lua-call".to_string(), id.to_string()]]
}

fn client_less_ctx() -> ClientCtx {
    ClientCtx {
        cwd: PathBuf::from("/"),
        pane: None,
        env: Vec::new(),
        client: None,
        dialect: Dialect::Grofe,
        request: Default::default(),
    }
}

/// Adds `line` to the server's message log (`show-messages`) and stderr.
fn log(state: &mut State, line: String) {
    eprintln!("grofe {line}");
    clients::log_error(state, "lua", &line);
}

impl Host for StateHost<'_> {
    fn run(&mut self, argv: Vec<String>) -> Result<String, String> {
        dispatch::run_split(self.state, &self.ctx, &argv)
    }

    fn expand(&mut self, format: &str, target: Option<&str>) -> Result<String, String> {
        let mut ctx = self.ctx.clone();
        if let Some(t) = target {
            let target = cmd::target::parse_target(t).map_err(|e| e.to_string())?;
            ctx.pane = Some(cmd::Resolve::resolve_pane(
                &self.state.model,
                &target,
                &self.ctx,
            )?);
        }
        Ok(prompt::expand_command(self.state, &ctx, format))
    }

    fn bind(
        &mut self,
        table: &str,
        key: &str,
        id: u32,
        desc: Option<String>,
        repeat: bool,
    ) -> Result<(), String> {
        let key = parse_key(key)?;
        let mut b = Binding::new(lua_call(id), repeat, BindingSource::Plugin(PLUGIN.into()));
        b.shown = desc.clone();
        b.desc = desc;
        let tables = &mut self.state.registry.key_tables;
        let previous = tables.exact(table, &key).cloned();
        tables.insert(table, key, b);
        let table = table.to_string();
        self.replaced.insert(
            id,
            Replaced {
                table,
                key,
                previous,
            },
        );
        Ok(())
    }

    fn subscribe(&mut self, event: &str, id: u32) -> Result<(), String> {
        self.async_
            .subscribe(&mut self.state.events, &self.state.jobs, event, id)
    }

    fn every(&mut self, ms: u64, id: u32) -> Result<(), String> {
        self.async_.every(&self.state.jobs, ms, id);
        Ok(())
    }

    fn system(
        &mut self,
        argv: Vec<String>,
        cwd: Option<String>,
        timeout_ms: u64,
        id: u32,
    ) -> Result<(), String> {
        self.async_.system(
            &self.state.jobs,
            argv,
            cwd.map(PathBuf::from),
            Duration::from_millis(timeout_ms),
            id,
        );
        Ok(())
    }

    fn cancel(&mut self, id: u32) {
        self.async_.cancel(id);
        let Some(r) = self.replaced.remove(&id) else {
            return;
        };
        if restore_key(self.state, id, &r) {
            return;
        }
        // A later bind took the key over this one: it gets what this one
        // had replaced, for when it is cancelled in turn.
        let next = self.replaced.values_mut().find(|n| {
            n.table == r.table
                && n.key == r.key
                && n.previous.as_ref().is_some_and(|p| p.cmds == lua_call(id))
        });
        if let Some(next) = next {
            next.previous = r.previous;
        }
    }

    fn notify(&mut self, text: &str, pane: Option<&str>, level: &str) -> Result<(), String> {
        let target = match pane {
            Some(p) => cmd::target::parse_target(p).map_err(|e| e.to_string())?,
            None => cmd::target::Target::Current,
        };
        let pane = cmd::Resolve::resolve_pane(&self.state.model, &target, &self.ctx)
            .map_err(|e| format!("no pane to notify for: {e}"))?;
        let uid = match self.state.model.panes.get(&pane) {
            Some(p) => p.uid.clone(),
            None => return Err(format!("no pane {pane}")),
        };
        self.state
            .notify
            .post(pane, &uid, level, text, std::time::Instant::now());
        clients::mark_all_dirty(self.state);
        Ok(())
    }

    fn log(&mut self, msg: &str) {
        log(self.state, format!("lua: {msg}"));
    }
}

/// Runs `f` with the runtime (made on first use) and a host over `state`.
/// `Err` when this is a nested call or the runtime cannot be made.
fn with_host<R>(
    state: &mut State,
    ctx: ClientCtx,
    f: impl FnOnce(&Runtime, &mut dyn Host) -> Result<R, LuaError>,
) -> Result<R, LuaError> {
    let fail = |m: &str| LuaError {
        who: "lua".into(),
        message: m.into(),
    };
    let Some(_guard) = InCall::enter() else {
        return Err(fail("nested call"));
    };
    let mut lua = match state.lua.take() {
        Some(l) => l,
        None => {
            let rt = Runtime::new(Limits::default())?;
            api::install(&rt)?;
            LuaHost {
                rt,
                async_: Async::default(),
                replaced: HashMap::new(),
            }
        }
    };
    let result = f(
        &lua.rt,
        &mut StateHost {
            state,
            async_: &mut lua.async_,
            replaced: &mut lua.replaced,
            ctx,
        },
    );
    if state.lua.is_none() {
        state.lua = Some(lua);
    }
    result
}

fn log_error(state: &mut State, e: &LuaError) {
    log(state, format!("lua: {e}"));
}

/// Drops the runtime with its timers and subscriptions, and every key Lua
/// bound.
pub(crate) fn reset(state: &mut State) {
    if IN_CALL.with(Cell::get) {
        DEFERRED.with(|d| *d.borrow_mut() = Some(Deferred::default()));
    }
    if let Some(mut lua) = state.lua.take() {
        lua.async_.cancel_all();
    }
    let ours = BindingSource::Plugin(PLUGIN.into());
    unbind_where(state, |b| b.source == ours);
}

/// Takes bind `id` off its key, if it still holds it, and puts back what
/// the key had before; whether it held the key.
fn restore_key(state: &mut State, id: u32, r: &Replaced) -> bool {
    let tables = &mut state.registry.key_tables;
    if tables
        .exact(&r.table, &r.key)
        .is_some_and(|b| b.cmds == lua_call(id))
    {
        tables.unbind(&r.table, &r.key);
        if let Some(previous) = r.previous.clone() {
            tables.insert(&r.table, r.key, previous);
        }
        return true;
    }
    false
}

/// Removes every key binding `drop_it` accepts.
fn unbind_where(state: &mut State, drop_it: impl Fn(&Binding) -> bool) {
    let tables = &mut state.registry.key_tables;
    let names: Vec<String> = tables.table_names().map(String::from).collect();
    for table in names {
        for key in tables.keys(&table) {
            if tables.exact(&table, &key).is_some_and(&drop_it) {
                tables.unbind(&table, &key);
            }
        }
    }
}

/// `reset`, then points `require` at `config_dir` on the fresh runtime, so
/// packs loaded before `init.luau` can require from it too.
pub fn reset_at(state: &mut State, config_dir: &Path) {
    reset(state);
    if !deferring() && !config_dir.as_os_str().is_empty() {
        set_folder(state, config_dir);
    }
}

fn set_folder(state: &mut State, config_dir: &Path) {
    let r = with_host(state, client_less_ctx(), |rt, _| {
        require::set_dir(rt.lua(), config_dir).map_err(|e| LuaError {
            who: "require".into(),
            message: e.to_string(),
        })
    });
    if let Err(e) = r {
        log_error(state, &e);
    }
}

/// `init.luau`, else `init.lua`, in `dir`.
fn init_file(dir: &Path) -> Option<PathBuf> {
    ["init.luau", "init.lua"]
        .iter()
        .map(|f| dir.join(f))
        .find(|p| p.is_file())
}

/// Loads pack `name` from `dir` with its merged `options`.
pub(crate) fn load_pack(
    state: &mut State,
    name: &str,
    dir: &Path,
    options: &serde_json::Value,
) -> Result<(), String> {
    if deferring() {
        DEFERRED.with(|d| {
            if let Some(d) = d.borrow_mut().as_mut() {
                d.packs
                    .push((name.to_string(), dir.to_path_buf(), options.clone()));
            }
        });
        return Ok(());
    }
    let file = init_file(dir).ok_or_else(|| format!("{name}: no init.luau"))?;
    let src = std::fs::read_to_string(&file).map_err(|e| format!("{name}: {e}"))?;
    with_host(state, client_less_ctx(), |rt, host| {
        api::enter(rt, host, name, |lua| {
            pack::register(lua, name, dir, options)?;
            let env = pack::env_of(lua, name)?;
            let value = api::in_pack(lua, || {
                lua.load(&src)
                    .set_name(name)
                    .set_environment(env)
                    .eval::<mlua::Value>()
            })?;
            api::set_plugin(rt, name, value).map_err(mlua::Error::external)
        })
    })
    .map_err(|e| e.to_string())
}

pub(crate) fn load_init(state: &mut State, config_dir: &Path) {
    if deferring() {
        let Some(d) = DEFERRED.with(|d| d.borrow_mut().take()) else {
            return;
        };
        let dir = config_dir.to_path_buf();
        let job: crate::server::Job = Box::new(move |state| run_deferred(state, d, dir));
        if state.jobs.try_send(job).is_err() {
            log(
                state,
                "lua: reload inside a call could not be queued".into(),
            );
        }
        return;
    }
    if config_dir.as_os_str().is_empty() {
        return;
    }
    let Some(file) = init_file(config_dir) else {
        return set_folder(state, config_dir);
    };
    let who = file
        .file_name()
        .map_or_else(|| "init".into(), |n| n.to_string_lossy().into_owned());
    let src = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => return log(state, format!("lua: {who}: {e}")),
    };
    let r = with_host(state, client_less_ctx(), |rt, host| {
        api::enter(rt, host, &who, |lua| {
            require::set_dir(lua, config_dir)?;
            lua.load(&src).set_name(&who).exec()
        })
    });
    if let Err(e) = r {
        log_error(state, &e);
    }
}

pub(crate) fn run_chunk(state: &mut State, ctx: &ClientCtx, src: &str) -> Result<String, String> {
    with_host(state, ctx.clone(), |rt, host| {
        api::enter(rt, host, "lua", |lua| {
            let values = lua.load(src).set_name("lua").eval::<mlua::MultiValue>()?;
            let parts: Result<Vec<String>, _> = values.iter().map(|v| v.to_string()).collect();
            Ok(parts?.join(" "))
        })
    })
    .map_err(|e| e.to_string())
}

pub(crate) fn call(state: &mut State, id: u32, payload: Payload) {
    let r = with_host(state, client_less_ctx(), |rt, host| {
        api::call(rt, host, id, payload)
    });
    if let Err(e) = r {
        log_error(state, &e);
    }
}

#[cfg(test)]
mod tests;
