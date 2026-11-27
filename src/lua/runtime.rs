use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// CPU time running Lua. Host calls and compiling stop the clock.
    pub budget: Duration,
    /// Wall-clock time for the whole call, host calls included: a loop of
    /// host calls spends almost none of `budget`, and must still end.
    pub wall: Duration,
    pub memory: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            budget: Duration::from_millis(50),
            // ponytail: one fixed ceiling for every call; a slow disk or
            // a busy machine near it would need it per call kind.
            wall: Duration::from_secs(5),
            memory: 64 << 20,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaError {
    pub who: String,
    pub message: String,
}

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.who, self.message)
    }
}

impl std::error::Error for LuaError {}

pub struct Runtime {
    lua: mlua::Lua,
    limits: Limits,
    clock: Arc<Mutex<Option<Clock>>>,
}

/// The running call's limits.
#[derive(Debug, Clone, Copy)]
struct Clock {
    /// The thread's CPU time when the call began.
    cpu_start: Duration,
    /// The CPU time host calls and compiles took since.
    paused: Duration,
    /// Until then the budget cannot be spent: CPU time grows no faster than
    /// wall time, so the interrupt reads it only from here on.
    check_at: Instant,
    /// When `wall` runs out.
    wall: Instant,
    /// Interrupts so far: one that came during a host call means Lua ran
    /// inside it (a metamethod on an argument), and that call is not paused.
    steps: u64,
    /// The budget ran out: every later step raises, whatever a host call
    /// that caught the error gives back.
    spent: bool,
}

impl Clock {
    /// The CPU time the call has spent running Lua.
    fn used(&self) -> Duration {
        thread_cpu_time().saturating_sub(self.cpu_start + self.paused)
    }
}

/// The running call's clock, where code holding only the `Lua` finds it.
struct Shared(Arc<Mutex<Option<Clock>>>);

fn lock(d: &Mutex<Option<Clock>>) -> std::sync::MutexGuard<'_, Option<Clock>> {
    d.lock().unwrap_or_else(|e| e.into_inner())
}

/// This thread's CPU time: a busy machine that keeps the thread waiting, or
/// a disk it waits on, spends none of the budget. Zero if it cannot be read.
pub(crate) fn thread_cpu_time() -> Duration {
    use nix::libc;
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec to write to.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) } != 0 {
        // ponytail: no thread clock (never seen on Linux or macOS) means no
        // budget; the wall limit still ends the call.
        return Duration::ZERO;
    }
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

impl Runtime {
    pub fn new(limits: Limits) -> Result<Runtime, LuaError> {
        let setup = |e: mlua::Error| LuaError {
            who: "runtime".into(),
            message: e.to_string(),
        };
        let lua = mlua::Lua::new();
        let clock: Arc<Mutex<Option<Clock>>> = Arc::new(Mutex::new(None));
        let c = clock.clone();
        lua.set_interrupt(move |_| {
            let mut guard = lock(&c);
            let Some(clock) = guard.as_mut() else {
                return Ok(mlua::VmState::Continue);
            };
            clock.steps = clock.steps.wrapping_add(1);
            let now = Instant::now();
            if clock.spent || now >= clock.check_at {
                let used = clock.used();
                if clock.spent || used >= limits.budget {
                    clock.spent = true;
                    return Err(mlua::Error::runtime(format!(
                        "over its {} ms budget",
                        limits.budget.as_millis()
                    )));
                }
                clock.check_at = now + (limits.budget - used);
            }
            if now >= clock.wall {
                return Err(mlua::Error::runtime(format!(
                    "over its {} ms limit, host calls included",
                    limits.wall.as_millis()
                )));
            }
            Ok(mlua::VmState::Continue)
        });
        lua.globals()
            .set("loadstring", mlua::Value::Nil)
            .map_err(setup)?;
        super::require::install(&lua).map_err(setup)?;
        lua.sandbox(true).map_err(setup)?;
        lua.set_memory_limit(limits.memory).map_err(setup)?;
        lua.set_app_data(Shared(clock.clone()));
        Ok(Runtime { lua, limits, clock })
    }

    /// Runs `f` under the limits. A nested call runs under the outer one's.
    pub fn guarded<R>(
        &self,
        who: &str,
        f: impl FnOnce(&mlua::Lua) -> mlua::Result<R>,
    ) -> Result<R, LuaError> {
        let fresh = lock(&self.clock).is_none();
        if fresh {
            let now = Instant::now();
            *lock(&self.clock) = Some(Clock {
                cpu_start: thread_cpu_time(),
                paused: Duration::ZERO,
                check_at: now + self.limits.budget,
                wall: now + self.limits.wall,
                steps: 0,
                spent: false,
            });
        }
        let result = f(&self.lua);
        if fresh {
            *lock(&self.clock) = None;
        }
        result.map_err(|e| LuaError {
            who: who.to_string(),
            message: clean(&e),
        })
    }

    pub fn exec(&self, who: &str, src: &str) -> Result<(), LuaError> {
        self.guarded(who, |lua| {
            compile(lua, lua.load(src).set_name(who))?.call(())
        })
    }

    pub fn lua(&self) -> &mlua::Lua {
        &self.lua
    }
}

/// Runs `f`, host work a Lua call asked for, with the budget's clock
/// stopped, unless Lua ran inside it. The wall limit runs on, so a loop of
/// host calls still ends.
pub fn off_budget<R>(lua: &mlua::Lua, f: impl FnOnce() -> R) -> R {
    let shared = lua.app_data_ref::<Shared>().map(|s| s.0.clone());
    let steps = shared.as_ref().and_then(|c| lock(c).map(|c| c.steps));
    let start = thread_cpu_time();
    let r = f();
    if let Some(shared) = &shared
        && let Some(clock) = lock(shared).as_mut()
        && Some(clock.steps) == steps
    {
        clock.paused += thread_cpu_time().saturating_sub(start);
    }
    r
}

/// A Rust function for the `grofe` API, made as `create_function` makes one,
/// that runs [`off_budget`]: the budget measures running Lua, not the host
/// work (a reload, an fsync, a spawn) a call asks for.
pub fn host_function<A, R>(
    lua: &mlua::Lua,
    f: impl Fn(&mlua::Lua, A) -> mlua::Result<R> + mlua::MaybeSend + 'static,
) -> mlua::Result<mlua::Function>
where
    A: mlua::FromLuaMulti,
    R: mlua::IntoLuaMulti,
{
    lua.create_function(move |lua, args: A| off_budget(lua, || f(lua, args)))
}

/// Compiles `chunk` [`off_budget`]: only running the code counts. A debug
/// build takes ~30 ms to compile 64 KiB of Luau, most of the budget.
pub fn compile(lua: &mlua::Lua, chunk: mlua::chunk::Chunk) -> mlua::Result<mlua::Function> {
    off_budget(lua, || chunk.into_function())
}

/// [`compile`] for a chunk run as `Chunk::eval` runs one: as an expression
/// when `src` is one, else as a block.
pub fn compile_eval(
    lua: &mlua::Lua,
    src: &[u8],
    name: &str,
    env: Option<mlua::Table>,
) -> mlua::Result<mlua::Function> {
    let chunk = |src: Vec<u8>| {
        let c = lua.load(src).set_name(name);
        match &env {
            Some(env) => c.set_environment(env.clone()),
            None => c,
        }
    };
    let expression = [b"return ".as_slice(), src].concat();
    compile(lua, chunk(expression)).or_else(|_| compile(lua, chunk(src.to_vec())))
}

/// The innermost error text, without mlua's wrapper and traceback.
fn clean(e: &mlua::Error) -> String {
    match e {
        mlua::Error::CallbackError { cause, .. } => clean(cause),
        mlua::Error::RuntimeError(m) => m.lines().next().unwrap_or(m).to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn rt() -> Runtime {
        Runtime::new(Limits::default()).unwrap()
    }
    #[test]
    fn a_spin_ends_at_the_budget() {
        let r = rt();
        let t = Instant::now();
        let e = r.exec("spin", "while true do end").unwrap_err();
        // Not before the budget (the deadline is honoured), and finite (unbudgeted it never ends).
        // The upper bound is loose so a descheduled runner cannot fail it.
        assert!(t.elapsed() >= Limits::default().budget, "{:?}", t.elapsed());
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        assert_eq!(e.who, "spin");
        assert!(e.message.contains("budget"), "{e}");
    }
    #[test]
    fn pcall_cannot_swallow_the_budget() {
        let r = rt();
        let t = Instant::now();
        assert!(
            r.exec(
                "p",
                "while true do pcall(function() while true do end end) end"
            )
            .is_err()
        );
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    }
    #[test]
    fn the_next_call_after_an_abort_runs() {
        let r = rt();
        let _ = r.exec("a", "while true do end");
        r.exec("b", "y = 2").unwrap();
        assert_eq!(r.lua().globals().get::<i64>("y").unwrap(), 2);
    }
    #[test]
    fn no_blocking_stdlib() {
        rt().exec(
            "s",
            "assert(io == nil and os.execute == nil and loadstring == nil)",
        )
        .unwrap();
    }
    #[test]
    fn memory_is_capped() {
        let r = Runtime::new(Limits {
            memory: 8 << 20,
            budget: Duration::from_secs(5),
            ..Limits::default()
        })
        .unwrap();
        let e = r
            .exec(
                "m",
                "local t = {} for i = 1, 1e6 do t[i] = string.rep('x', 65536) .. i end",
            )
            .unwrap_err();
        assert!(e.message.contains("memory"), "{e}");
    }
    #[test]
    fn a_syntax_error_names_the_chunk() {
        assert_eq!(rt().exec("init.luau", "x =").unwrap_err().who, "init.luau");
    }
    #[test]
    fn test_nested_guarded_keeps_outer_deadline() {
        let r = rt();
        // Baseline: loop iterations a spin gets from one full, fresh budget.
        r.exec("base", "n = 0 while true do n = n + 1 end")
            .unwrap_err();
        let fresh: f64 = r.lua().globals().get("n").unwrap();
        r.guarded("outer", |lua| {
            // Past the outer budget: a fresh inner budget would let the spin run as long as
            // the baseline did. Counting iterations instead of time ignores runner speed.
            let _ = lua.load("while true do end").exec();
            let inner = r
                .exec("inner", "n = 0 while true do n = n + 1 end")
                .unwrap_err();
            let n: f64 = r.lua().globals().get("n").unwrap();
            assert!(
                n * 10.0 < fresh,
                "inner ran {n} iterations, fresh budget {fresh}"
            );
            assert!(inner.message.contains("budget"), "{inner}");
            Ok(())
        })
        .unwrap();
    }

    /// Runs `src`, checks it ended fast with an error containing `want`, then that the VM still runs.
    fn aborts(src: &str, want: &str) {
        let r = Runtime::new(Limits::default()).unwrap();
        let t = crate::lua::runtime::thread_cpu_time();
        let e = r.exec("p", src).unwrap_err();
        // Unbudgeted, these never return, so any finite bound proves the abort. CPU time, as
        // the budget is: a loaded runner stretches the wall time.
        let took = crate::lua::runtime::thread_cpu_time() - t;
        assert!(took < Duration::from_secs(2), "{src}: {took:?}");
        assert!(e.message.contains(want), "{src}: {e}");
        runs_after(&r);
    }

    /// Checks that a trivial call runs; a stale deadline fails at once, so only a full-budget
    /// miss is retried.
    fn runs_after(r: &Runtime) {
        let budget = Limits::default().budget;
        for _ in 0..5 {
            let t = Instant::now();
            match r.exec("next", "z = 1") {
                Ok(()) => return assert_eq!(r.lua().globals().get::<i64>("z").unwrap(), 1),
                Err(e) => assert!(
                    e.message.contains("budget") && t.elapsed() >= budget,
                    "{e} after {:?}",
                    t.elapsed()
                ),
            }
        }
        panic!("five trivial calls in a row ran over the budget");
    }
    #[test]
    fn deep_recursion_is_a_lua_error() {
        aborts(
            "local function f(n) return 1 + f(n + 1) end f(1)",
            "stack overflow",
        );
    }
    #[test]
    fn huge_allocations_are_errors() {
        // Not 2^31: Luau casts the count to int, which wraps negative on x86_64 and yields "".
        aborts("string.rep('x', 2^30 + 1)", "too large");
        aborts("string.rep('x', 1e9)", "memory");
        aborts("string.format('%999999999d', 1)", "invalid format");
        aborts("buffer.create(1e9)", "memory");
    }
    #[test]
    fn the_interrupt_fires_inside_the_pattern_matcher() {
        // Timing-free: the subject is built outside the budget and `done` is set only if the
        // matcher returns, so a budget error with `done` unset means it was interrupted inside.
        // Uninterrupted, the match never ends, the find takes ~2 s and the gsub runs out of memory.
        for (n, src) in [
            (20_000, "s:match('(.-)(.-)(.-)(.-)x') done = true"),
            (20_000_000, "s:find('%b()') done = true"),
            (
                1_000_000,
                "string.gsub(s, 'a', 'bbbbbbbbbbbbbbbbbbbbbbbbbbb') done = true",
            ),
        ] {
            let r = rt();
            let s = r.lua().create_string(vec![b'a'; n]).unwrap();
            r.lua().globals().set("s", s).unwrap();
            let e = r.exec("p", src).unwrap_err();
            assert!(e.message.contains("budget"), "{src}: {e}");
            let done: Option<bool> = r.lua().globals().get("done").unwrap();
            assert_eq!(done, None, "{src} ran to completion");
            runs_after(&r);
        }
    }
    #[test]
    fn spins_in_coroutines_sorts_and_metamethods_abort() {
        aborts(
            "coroutine.wrap(function() while true do end end)()",
            "budget",
        );
        aborts(
            "local c = coroutine.wrap(function() while true do coroutine.yield() end end) while true do c() end",
            "budget",
        );
        aborts(
            "table.sort({3,2,1,5,4}, function(a, b) while true do end end)",
            "budget",
        );
        aborts(
            "local t = setmetatable({}, {__index = function() while true do end end}) return t.x",
            "budget",
        );
        aborts(
            "xpcall(function() error('x') end, function() while true do end end)",
            "budget",
        );
    }
    #[test]
    fn the_memory_limit_still_applies_after_an_oom() {
        let r = Runtime::new(Limits {
            memory: 8 << 20,
            budget: Duration::from_secs(5),
            ..Limits::default()
        })
        .unwrap();
        for _ in 0..2 {
            let e = r.exec("m", "string.rep('x', 1e9)").unwrap_err();
            assert!(e.message.contains("memory"), "{e}");
            r.exec("ok", "local s = string.rep('x', 1e6)").unwrap();
        }
    }
    #[test]
    fn the_sandbox_is_closed() {
        let r = rt();
        for src in [
            "rawset(_G, 'zz', 1)",
            "string.zz = 1",
            "require('/etc/passwd')",
            "require('../../etc/passwd')",
            "dofile('/etc/passwd')",
            "collectgarbage('stop')",
            "collectgarbage('setpause', 1000000)",
            "assert(package == nil and io == nil and os.getenv == nil and os.exit == nil and os.remove == nil)",
        ] {
            let res = r.exec("s", src);
            if src.starts_with("assert") {
                res.unwrap();
            } else {
                assert!(res.is_err(), "{src} was allowed");
            }
        }
        // getfenv/setfenv only reach the sandbox env, never the shared read-only globals
        r.exec("f", "getfenv(1).leak = 1 setfenv(1, {})").unwrap();
        assert!(r.exec("g", "string.leak = 1").is_err());
    }
    // ponytail: one C call (a plain find or upper on a string up to the memory cap) is not
    // interruptible; its cost is bounded by the 64 MiB memory limit, measured at ~40 ms.
    #[test]
    fn a_single_uninterruptible_c_call_is_bounded_by_the_memory_cap() {
        let r = rt();
        let t = Instant::now();
        let _ = r.exec("c", "string.rep('a', 5e7):find('b')");
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    }
}
