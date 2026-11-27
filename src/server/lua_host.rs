//! The `Host` for the server's `State`: Luau's entry points on the actor.
//! `state.lua` is taken out of `State` for every call and put back after,
//! so the host can hold `&mut State`; a call that finds it taken is nested.

use crate::cmd;
use crate::lua::api::{self, Host, Payload, PromptJob, SystemJob};
use crate::lua::runtime::{Limits, LuaError, Runtime, compile, compile_eval};
use crate::lua::{pack, require};
use crate::protocol::{ClientCtx, Dialect};
use crate::server::agents::{headless, valid_kind};
use crate::server::events::Scope;
use crate::server::input::{Binding, BindingSource, Key, parse_key};
use crate::server::lua_async::Async;
use crate::server::{State, clients, dispatch, prompt};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const PLUGIN: &str = "lua";

pub struct LuaHost {
    rt: Runtime,
    async_: Async,
    /// Per bind id, the key it took and what held that key before.
    replaced: HashMap<u32, Replaced>,
}

/// The generation the next runtime gets. Callback ids restart at 1 with each
/// runtime, so a job queued by an earlier one must be told apart by this.
static GENERATION: AtomicU64 = AtomicU64::new(1);

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

#[cfg(test)]
thread_local! {
    /// The limits runtimes made on this thread get, for a test whose host
    /// work (a whole `reload`, say) a slow runner can push past the budget.
    static TEST_LIMITS: Cell<Option<Limits>> = const { Cell::new(None) };
}

/// Test hook: the budget in milliseconds, for integration tests whose host
/// work (durable `grofe.store` writes, say) a loaded runner can push past
/// the default. Test-only.
const TEST_BUDGET_ENV: &str = "GROFE_TEST_LUA_BUDGET_MS";

/// The limits a new runtime gets.
fn limits() -> Limits {
    #[cfg(test)]
    if let Some(l) = TEST_LIMITS.with(Cell::get) {
        return l;
    }
    let mut limits = Limits::default();
    if let Some(ms) = std::env::var(TEST_BUDGET_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
    {
        limits.budget = std::time::Duration::from_millis(ms);
    }
    limits
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
    /// No request owns this call: see [`with_client_less_host`].
    client_less: bool,
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
pub(crate) fn log(state: &mut State, line: String) {
    eprintln!("grofe {line}");
    clients::log_error(state, "lua", &line);
}

impl Host for StateHost<'_> {
    fn run(&mut self, argv: Vec<String>) -> Result<String, String> {
        let had_held = self.state.held.is_some();
        let result = dispatch::run_split(self.state, &self.ctx, &argv);
        if self.client_less {
            keep_replies(self.state, had_held);
        }
        result
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

    fn subscribe(&mut self, event: &str, id: u32, pane: Option<&str>) -> Result<(), String> {
        let scope = match pane {
            Some(p) => {
                let target = cmd::target::parse_target(p).map_err(|e| e.to_string())?;
                if !matches!(target, cmd::Target::Pane(_)) {
                    return Err(format!("grofe.on: pane = {p:?} is not a pane target"));
                }
                Some(Scope::resolve(&self.state.model, &self.ctx, &target)?)
            }
            None => None,
        };
        self.async_
            .subscribe(&mut self.state.events, &self.state.jobs, event, scope, id)?;
        Ok(())
    }

    fn every(&mut self, ms: u64, id: u32) -> Result<(), String> {
        self.async_.every(&self.state.jobs, ms, id);
        Ok(())
    }

    fn system(&mut self, job: SystemJob, id: u32) -> Result<(), String> {
        if let Some(a) = &job.agent {
            if !valid_kind(&a.kind) {
                return Err(format!(
                    "grofe.system: agent kind {:?} is not a name",
                    a.kind
                ));
            }
            headless::start(self.state, self.async_.generation(), id, &a.kind, &a.label);
        }
        self.async_.system(&self.state.jobs, job, id);
        Ok(())
    }

    fn agent_history(&mut self, pane: &str, n: usize) -> Result<Vec<String>, String> {
        let target = cmd::target::parse_target(pane).map_err(|e| e.to_string())?;
        let id = cmd::Resolve::resolve_pane(&self.state.model, &target, &self.ctx)?;
        Ok(self.state.agents.history(id, n))
    }

    fn agent_read(&mut self, pane: &str, n: usize) -> Result<String, String> {
        let target = cmd::target::parse_target(pane).map_err(|e| e.to_string())?;
        let id = cmd::Resolve::resolve_pane(&self.state.model, &target, &self.ctx)?;
        dispatch::agent_control::read_lines(self.state, id, n)
    }

    fn agent_prompt(&mut self, req: PromptJob, id: u32) -> Result<(), String> {
        let reply = (self.async_.generation(), id);
        dispatch::agent_control::prompt_lua(self.state, &self.ctx, &req, reply)
    }

    fn cancel(&mut self, id: u32) {
        dispatch::agent_control::cancel_lua(self.state, self.async_.generation(), id);
        self.async_.cancel(id);
        headless::cancel(self.state, self.async_.generation(), id);
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
        let model = &self.state.model;
        let pane = match pane {
            Some(p) => {
                let target = cmd::target::parse_target(p).map_err(|e| e.to_string())?;
                let id = cmd::Resolve::resolve_pane(model, &target, &self.ctx)
                    .map_err(|e| format!("no pane to notify for: {e}"))?;
                Some(id)
            }
            // The tray is server-wide: with no session there is no current
            // pane, and the entry is the server's own.
            None => {
                cmd::Resolve::resolve_pane(model, &cmd::target::Target::Current, &self.ctx).ok()
            }
        };
        let uid = match pane {
            Some(id) => match model.panes.get(&id) {
                Some(p) => p.uid.clone(),
                None => return Err(format!("no pane {id}")),
            },
            None => String::new(),
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
    client_less: bool,
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
            let rt = Runtime::new(limits())?;
            api::install(&rt)?;
            LuaHost {
                rt,
                async_: Async::new(GENERATION.fetch_add(1, Ordering::Relaxed)),
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
            client_less,
        },
    );
    if state.lua.is_none() {
        state.lua = Some(lua);
    }
    result
}

/// [`with_host`] for work no client request owns (timers, events, system
/// callbacks, init, packs): each command's deferred reply is kept (see
/// [`keep_replies`]) instead of left for the next request.
fn with_client_less_host<R>(
    state: &mut State,
    f: impl FnOnce(&Runtime, &mut dyn Host) -> Result<R, LuaError>,
) -> Result<R, LuaError> {
    with_host(state, client_less_ctx(), true, f)
}

/// After one command of client-less work: what it held back, and the reply
/// it will give later, would be taken by the next request (or by the request
/// `reload` loads init in) as its own. Each gets a task that keeps it alive,
/// so the held rest still runs, and logs a failure nobody else would show.
fn keep_replies(state: &mut State, had_held: bool) {
    if !had_held && let Some(held) = state.held.take() {
        dispatch::client::hold_rest(state, &client_less_ctx(), held, String::new(), None, None);
    }
    if let Some(reply) = state.deferred_reply.take()
        && let Ok(rt) = tokio::runtime::Handle::try_current()
    {
        let jobs = state.jobs.clone();
        rt.spawn(async move {
            if let Ok(Err(f)) = reply.await {
                let mut text = [f.err.trim_end(), f.out.trim_end()]
                    .iter()
                    .filter(|t| !t.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.is_empty() {
                    match f.status {
                        Some(code) => text = format!("returned {code}"),
                        None => return,
                    }
                }
                let job: crate::server::Job = Box::new(move |state| {
                    log(state, format!("lua: {text}"));
                });
                let _ = jobs.send(job).await;
            }
        });
    }
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
    headless::clear(state);
    dispatch::agent_control::clear_lua(state);
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
    let r = with_client_less_host(state, |rt, _| {
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
    with_client_less_host(state, |rt, host| {
        api::enter(rt, host, name, |lua| {
            pack::register(lua, name, dir, options)?;
            let env = pack::env_of(lua, name)?;
            let value = api::in_pack(lua, || {
                compile_eval(lua, src.as_bytes(), name, Some(env))?.call::<mlua::Value>(())
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
    let r = with_client_less_host(state, |rt, host| {
        api::enter(rt, host, &who, |lua| {
            require::set_dir(lua, config_dir)?;
            compile(lua, lua.load(&src).set_name(&who))?.call::<()>(())
        })
    });
    if let Err(e) = r {
        log_error(state, &e);
    }
}

pub(crate) fn run_chunk(state: &mut State, ctx: &ClientCtx, src: &str) -> Result<String, String> {
    with_host(state, ctx.clone(), false, |rt, host| {
        api::enter(rt, host, "lua", |lua| {
            let values =
                compile_eval(lua, src.as_bytes(), "lua", None)?.call::<mlua::MultiValue>(())?;
            let parts: Result<Vec<String>, _> = values.iter().map(|v| v.to_string()).collect();
            Ok(parts?.join(" "))
        })
    })
    .map_err(|e| e.to_string())
}

pub(crate) fn call(state: &mut State, id: u32, payload: Payload) {
    let r = with_host(state, client_less_ctx(), false, |rt, host| {
        api::call(rt, host, id, payload)
    });
    if let Err(e) = r {
        log_error(state, &e);
    }
}

/// Whether runtime generation `generation` is the one running: a job queued by an
/// earlier runtime names callbacks that are not this one's.
pub(crate) fn is_current(state: &State, generation: u64) -> bool {
    state
        .lua
        .as_ref()
        .is_some_and(|l| l.async_.generation() == generation)
}

/// The subscription behind callback `id` ended on the server's side (its
/// pane, window or session closed): its callback is freed.
pub(crate) fn forget_subscription(state: &mut State, id: u32) {
    if let Some(lua) = state.lua.as_mut()
        && lua.async_.forget(id)
    {
        api::release(&lua.rt, id);
    }
}

/// [`call`] for a timer, event or child-process callback: no request owns it.
pub(crate) fn call_job(state: &mut State, id: u32, payload: Payload) {
    let r = with_client_less_host(state, |rt, host| api::call(rt, host, id, payload));
    if let Err(e) = r {
        log_error(state, &e);
    }
}

/// [`call_job`] for a callback that runs once: it is freed after.
pub(crate) fn call_once(state: &mut State, id: u32, payload: Payload) {
    call_job(state, id, payload);
    if let Some(lua) = state.lua.as_ref() {
        api::release(&lua.rt, id);
    }
}

/// Frees callback `id` of runtime `generation` when its [`call_once`] job
/// could not be queued.
pub(crate) fn release_once(state: &State, generation: u64, id: u32) {
    if let Some(lua) = state
        .lua
        .as_ref()
        .filter(|l| l.async_.generation() == generation)
    {
        api::release(&lua.rt, id);
    }
}

#[cfg(test)]
mod tests;
