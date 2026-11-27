use super::*;
use crate::server::{test_ctx, test_state};
use std::time::{Duration, Instant};

fn messages(state: &State) -> String {
    clients::messages(state)
        .map(|(_, l)| l.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn run_chunk_returns_values() {
    let mut state = test_state();
    let ctx = test_ctx();
    assert_eq!(run_chunk(&mut state, &ctx, "return 1 + 1"), Ok("2".into()));
    assert_eq!(
        run_chunk(&mut state, &ctx, "return 1, 'a'"),
        Ok("1 a".into())
    );
}

#[test]
fn nested_lua_verb_is_an_error() {
    let mut state = test_state();
    let e = run_chunk(&mut state, &test_ctx(), "grofe.cmd{'lua','return 1'}").unwrap_err();
    assert!(e.contains("nested"), "{e}");
    assert_eq!(
        run_chunk(&mut state, &test_ctx(), "return 3"),
        Ok("3".into())
    );
}

#[test]
fn a_spin_returns_a_budget_error_fast() {
    let mut state = test_state();
    // The budget is CPU time: a loaded runner stretches the wall time.
    let t = crate::thread_cpu_time();
    let e = run_chunk(&mut state, &test_ctx(), "while true do end").unwrap_err();
    assert!(e.contains("budget"), "{e}");
    let took = crate::thread_cpu_time() - t;
    assert!(took < Duration::from_millis(200), "{took:?}");
}

#[test]
fn test_reset_unbinds_lua_keys() {
    let mut state = test_state();
    let before = state
        .registry
        .key_tables
        .exact("prefix", &parse_key("c").unwrap())
        .cloned();
    assert!(before.is_some());
    run_chunk(
        &mut state,
        &test_ctx(),
        "grofe.bind('prefix','g',function() end)",
    )
    .unwrap();
    let g = parse_key("g").unwrap();
    assert!(state.registry.key_tables.exact("prefix", &g).is_some());
    reset(&mut state);
    assert!(state.registry.key_tables.exact("prefix", &g).is_none());
    let after = state
        .registry
        .key_tables
        .exact("prefix", &parse_key("c").unwrap())
        .cloned();
    assert_eq!(before, after);
    assert!(state.lua.is_none());
}

#[test]
fn test_load_init_error_logs_and_continues() {
    let mut state = test_state();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("init.luau"), "error('boom')").unwrap();
    load_init(&mut state, dir.path());
    assert!(messages(&state).contains("boom"), "{}", messages(&state));
    assert_eq!(
        run_chunk(&mut state, &test_ctx(), "return 1"),
        Ok("1".into())
    );
}

#[test]
fn test_bind_shows_desc_in_list_keys() {
    let mut state = test_state();
    let ctx = test_ctx();
    run_chunk(
        &mut state,
        &ctx,
        "grofe.bind('prefix','g',function() end,{desc='Git popup'})",
    )
    .unwrap();
    let out = dispatch::run_split(&mut state, &ctx, &["list-keys".into()]).unwrap();
    assert!(out.contains("Git popup"), "{out}");
}

#[test]
fn a_failing_callback_names_itself() {
    let mut state = test_state();
    run_chunk(
        &mut state,
        &test_ctx(),
        "grofe.bind('prefix','g',function() while true do end end)",
    )
    .unwrap();
    call(&mut state, 1, Payload::None);
    assert!(
        messages(&state).contains("callback 1"),
        "{}",
        messages(&state)
    );
    call(&mut state, 99, Payload::None);
}

#[test]
fn test_reload_inside_lua_is_deferred() {
    let (mut state, mut rx) = crate::server::test_state_at(PathBuf::from("/nonexistent/x.sock"));
    let ctx = test_ctx();
    let r = run_chunk(&mut state, &ctx, "grofe.cmd{'reload'}");
    assert!(r.is_ok(), "{r:?}");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("init.luau"),
        "grofe.bind('prefix','g',function() end)",
    )
    .unwrap();
    {
        let _in_call = InCall::enter().unwrap();
        reset(&mut state);
        load_init(&mut state, dir.path());
    }
    let g = parse_key("g").unwrap();
    assert!(state.registry.key_tables.exact("prefix", &g).is_none());
    while let Ok(job) = rx.try_recv() {
        job(&mut state);
    }
    assert!(state.registry.key_tables.exact("prefix", &g).is_some());
}

#[test]
fn load_pack_stores_the_return_value() {
    let mut state = test_state();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("init.luau"), "return { x = 1 }").unwrap();
    load_pack(&mut state, "p", dir.path(), &serde_json::json!({})).unwrap();
    let r = run_chunk(&mut state, &test_ctx(), "return grofe.plugin('p').x");
    assert_eq!(r, Ok("1".into()));
}
/// A config folder with an `init.luau` (empty unless `init` says more)
/// and the given `(relative path, source)` files.
fn folder(init: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("init.luau"), init).unwrap();
    for (rel, src) in files {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    }
    dir
}

fn global(state: &mut State, name: &str) -> String {
    let args = ["show-options".into(), "-gqv".into(), name.to_string()];
    dispatch::run_split(state, &test_ctx(), &args)
        .unwrap()
        .trim()
        .to_string()
}

fn require(state: &mut State, name: &str) -> Result<String, String> {
    run_chunk(state, &test_ctx(), &format!("return require({name:?})"))
}

/// t7 (A7): a module required twice in one load runs once and returns
/// the same table.
#[test]
fn a_module_required_twice_runs_once_and_is_the_same_table() {
    let dir = folder(
        "local a = require('counter')\n\
         local b = require('counter')\n\
         grofe.cmd{'set-option','-g','@same',tostring(a == b)}\n\
         grofe.cmd{'set-option','-g','@n',tostring(a.n)}",
        &[(
            "counter.luau",
            "grofe.cmd{'set-option','-ga','@runs','x'}\nreturn { n = 1 }",
        )],
    );
    let mut state = test_state();
    load_init(&mut state, dir.path());
    assert_eq!(global(&mut state, "@runs"), "x", "{}", messages(&state));
    assert_eq!(global(&mut state, "@same"), "true");
    assert_eq!(global(&mut state, "@n"), "1");
}

/// t7 (A7): each reload starts with an empty cache.
#[test]
fn a_reload_runs_the_module_again() {
    let dir = folder(
        "require('counter')",
        &[(
            "counter.luau",
            "grofe.cmd{'set-option','-ga','@runs','x'}\nreturn 1",
        )],
    );
    let mut state = test_state();
    load_init(&mut state, dir.path());
    reset(&mut state);
    load_init(&mut state, dir.path());
    assert_eq!(global(&mut state, "@runs"), "xx", "{}", messages(&state));
}

/// t8 (A6): the lookup order, `./`, dotted names and the return value.
#[test]
fn require_finds_luau_then_lua_then_init_files() {
    let dir = folder(
        "",
        &[
            ("both.luau", "return 'luau'"),
            ("both.lua", "return 'lua'"),
            ("only.lua", "return 'lua'"),
            ("pkg/init.luau", "return 'pkg-luau'"),
            ("pkg2/init.lua", "return 'pkg2-lua'"),
            ("ui/status.luau", "return 'ui'"),
        ],
    );
    let mut state = test_state();
    load_init(&mut state, dir.path());
    let got = |s: &mut State, n: &str| require(s, n);
    assert_eq!(got(&mut state, "both"), Ok("luau".into()));
    assert_eq!(got(&mut state, "only"), Ok("lua".into()));
    assert_eq!(got(&mut state, "pkg"), Ok("pkg-luau".into()));
    assert_eq!(got(&mut state, "pkg2"), Ok("pkg2-lua".into()));
    assert_eq!(got(&mut state, "ui.status"), Ok("ui".into()));
    assert_eq!(got(&mut state, "./both"), Ok("luau".into()));
}

/// t8 (A8): these names are Lua errors.
#[test]
fn require_refuses_dotdot_absolute_empty_and_extension() {
    let dir = folder("", &[("keymaps.luau", "return 1")]);
    let mut state = test_state();
    load_init(&mut state, dir.path());
    for name in ["..", "a..b", "/etc/passwd", "", "keymaps.luau"] {
        let r = require(&mut state, name);
        assert!(r.is_err(), "{name:?} loaded: {r:?}");
    }
    assert_eq!(require(&mut state, "keymaps"), Ok("1".into()));
}

/// t8 (A8): a missing module names itself and the paths tried.
#[test]
fn a_missing_module_lists_the_paths_tried() {
    let dir = folder("", &[]);
    let mut state = test_state();
    load_init(&mut state, dir.path());
    let e = require(&mut state, "nomod").unwrap_err();
    for part in ["nomod", "nomod.luau", "nomod.lua", "nomod/init.luau"] {
        assert!(e.contains(part), "{part} missing from: {e}");
    }
}

/// t8 (A8): a require cycle is an error, not a hang.
#[test]
fn a_require_cycle_is_an_error() {
    let dir = folder(
        "",
        &[
            ("a.luau", "return require('b')"),
            ("b.luau", "return require('a')"),
        ],
    );
    let mut state = test_state();
    load_init(&mut state, dir.path());
    let e = require(&mut state, "a").unwrap_err();
    assert!(e.contains("cycle"), "{e}");
}

/// t8 (A8): the 50 ms budget covers a required module.
#[test]
fn a_required_module_that_spins_hits_the_budget() {
    let dir = folder("", &[("spin.luau", "while true do end")]);
    let mut state = test_state();
    load_init(&mut state, dir.path());
    // The budget is CPU time: a loaded runner stretches the wall time.
    let t = crate::thread_cpu_time();
    let e = require(&mut state, "spin").unwrap_err();
    assert!(e.contains("budget"), "{e}");
    let took = crate::thread_cpu_time() - t;
    assert!(took < Duration::from_millis(500), "{took:?}");
}

/// t8 (A8): a module that errors on load is an error naming it.
#[test]
fn a_module_that_errors_on_load_is_an_error() {
    let dir = folder("", &[("bad.luau", "error('kaboom')")]);
    let mut state = test_state();
    load_init(&mut state, dir.path());
    let e = require(&mut state, "bad").unwrap_err();
    assert!(e.contains("kaboom"), "{e}");
}

/// t8 (ruling): a symlinked module file outside the folder loads.
#[test]
fn a_symlinked_module_outside_the_folder_loads() {
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("real.luau"), "return 'linked'").unwrap();
    let dir = folder("", &[]);
    std::os::unix::fs::symlink(
        outside.path().join("real.luau"),
        dir.path().join("ext.luau"),
    )
    .unwrap();
    let mut state = test_state();
    load_init(&mut state, dir.path());
    assert_eq!(require(&mut state, "ext"), Ok("linked".into()));
}

/// t8 (A9): `require` works in a callback after init returned.
#[test]
fn require_works_in_a_callback_after_init() {
    let dir = folder(
        "grofe.bind('prefix','g',function() require('late') end)",
        &[(
            "late.luau",
            "grofe.cmd{'set-option','-g','@late','yes'}\nreturn 1",
        )],
    );
    let mut state = test_state();
    load_init(&mut state, dir.path());
    assert_eq!(global(&mut state, "@late"), "");
    call(&mut state, 1, Payload::None);
    assert_eq!(global(&mut state, "@late"), "yes", "{}", messages(&state));
}

/// A pack's callback loads modules from the pack's folder, not the config
/// folder.
#[test]
fn a_pack_callback_requires_from_its_own_folder() {
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(
        pack.path().join("init.luau"),
        "grofe.bind('prefix','p',function() grofe.cmd{'set-option','-g','@pack',require('m')} end)",
    )
    .unwrap();
    std::fs::write(pack.path().join("m.luau"), "return 'pack'").unwrap();
    let dir = folder("", &[("m.luau", "return 'folder'")]);
    let mut state = test_state();
    load_pack(&mut state, "p", pack.path(), &serde_json::json!({})).unwrap();
    load_init(&mut state, dir.path());
    call(&mut state, 1, Payload::None);
    assert_eq!(global(&mut state, "@pack"), "pack", "{}", messages(&state));
}

/// An empty folder (no config folder) sets nothing; `require` is a plain
/// Lua error, not a lookup relative to the cwd.
#[test]
fn require_without_a_folder_is_an_error() {
    let mut state = test_state();
    load_init(&mut state, Path::new(""));
    let e = require(&mut state, "m").unwrap_err();
    assert!(e.contains("no config folder"), "{e}");
}

/// A9: the folder is set even when there is no init file.
#[test]
fn require_works_without_an_init_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("m.luau"), "return 'm'").unwrap();
    let mut state = test_state();
    load_init(&mut state, dir.path());
    assert_eq!(require(&mut state, "m"), Ok("m".into()));
}

/// A pack's top-level `require` loads from its own folder, and `init.luau`
/// from the config folder, whichever loads first.
#[test]
fn a_pack_and_init_require_from_their_own_folders_in_the_reload_order() {
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(pack.path().join("init.luau"), "return { m = require('m') }").unwrap();
    std::fs::write(pack.path().join("m.luau"), "return 'pack'").unwrap();
    let dir = folder(
        "grofe.cmd{'set-option','-g','@init',require('m')}",
        &[(
            "m.luau",
            "grofe.cmd{'set-option','-ga','@runs','x'}\nreturn 'folder'",
        )],
    );
    let mut state = test_state();
    reset_at(&mut state, dir.path());
    load_pack(&mut state, "p", pack.path(), &serde_json::json!({})).unwrap();
    load_init(&mut state, dir.path());
    let r = run_chunk(&mut state, &test_ctx(), "return grofe.plugin('p').m");
    assert_eq!(r, Ok("pack".into()), "{}", messages(&state));
    assert_eq!(global(&mut state, "@init"), "folder");
    assert_eq!(global(&mut state, "@runs"), "x");
}

#[tokio::test]
async fn a_subscription_is_freed_when_its_pane_closes() {
    let (mut state, mut rx) = crate::server::test_state_at(PathBuf::from("/nonexistent/x.sock"));
    let ctx = test_ctx();
    for words in [
        &["new-session", "-d", "-s", "a", "sleep 600"][..],
        &["new-window", "-t", "a", "sleep 600"][..],
    ] {
        let argv: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        dispatch::run(&mut state, &ctx, &argv).unwrap();
    }
    let pane = state.model.panes.keys().copied().last().unwrap();
    let code = format!("grofe.on('pane-closed', function() end, {{ pane = '{pane}' }})");
    run_chunk(&mut state, &ctx, &code).unwrap();
    assert_eq!(state.lua.as_ref().map(|l| l.async_.len()), Some(1));
    let kill = ["kill-pane".to_string(), "-t".to_string(), pane.to_string()];
    dispatch::run(&mut state, &ctx, &kill).unwrap();
    state.events.flush(&state.model);
    // The event, then the end of the subscription.
    for _ in 0..2 {
        let job = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        job(&mut state);
    }
    let lua = state.lua.as_ref().unwrap();
    assert_eq!(lua.async_.len(), 0);
    let callbacks: mlua::Table = lua
        .rt
        .lua()
        .named_registry_value("grofe_callbacks")
        .unwrap();
    assert!(matches!(
        callbacks.get::<mlua::Value>(1),
        Ok(mlua::Value::Nil)
    ));
    assert!(!state.events.named("pane-closed"));
}

fn callback_freed(state: &State, id: u32) -> bool {
    let lua = state.lua.as_ref().unwrap();
    let callbacks: mlua::Table = lua
        .rt
        .lua()
        .named_registry_value("grofe_callbacks")
        .unwrap();
    matches!(callbacks.get::<mlua::Value>(id), Ok(mlua::Value::Nil))
}

#[tokio::test]
async fn the_on_line_callback_is_freed_when_the_job_ends_or_is_cancelled() {
    let (mut state, mut rx) = crate::server::test_state_at(PathBuf::from("/nonexistent/x.sock"));
    let ctx = test_ctx();
    let code = "j = grofe.system({'sleep','30'}, function() end, {on_line = function() end})";
    run_chunk(&mut state, &ctx, code).unwrap();
    assert!(!callback_freed(&state, 1));
    run_chunk(&mut state, &ctx, "grofe.cancel(j)").unwrap();
    assert!(callback_freed(&state, 1) && callback_freed(&state, 2));

    let code = "grofe.system({'true'}, function() end, {on_line = function() end})";
    run_chunk(&mut state, &ctx, code).unwrap();
    assert!(!callback_freed(&state, 3));
    let job = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    job(&mut state);
    assert!(callback_freed(&state, 3));
}

#[tokio::test]
async fn jobs_queued_before_a_reload_are_ignored_by_the_new_runtime() {
    let (mut state, mut rx) = crate::server::test_state_at(PathBuf::from("/nonexistent/x.sock"));
    let ctx = test_ctx();
    // Ids 1 and 2 go to the lines and the end of a job that prints a line, and
    // a last one with no newline.
    run_chunk(
        &mut state,
        &ctx,
        "grofe.system({'sh','-c','echo a; printf b'}, function() old = 1 end, \
         {on_line = function() old = 1 end})",
    )
    .unwrap();
    let old = state.lua.as_ref().unwrap().async_.generation();
    // Both lines and the end are queued, but have not run yet.
    let mut queued = Vec::new();
    for _ in 0..3 {
        let job = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        queued.push(job);
    }
    reset(&mut state);
    // The new runtime hands out callback ids 1 and 2 again, to timers.
    run_chunk(
        &mut state,
        &ctx,
        "grofe.every(60000, function() new = 1 end) grofe.every(60000, function() new = 1 end)",
    )
    .unwrap();
    assert_ne!(state.lua.as_ref().unwrap().async_.generation(), old);
    for job in queued {
        job(&mut state);
    }
    crate::server::lua_async::guarded(old, |state| forget_subscription(state, 1))(&mut state);
    assert_eq!(
        run_chunk(&mut state, &ctx, "return tostring(new)"),
        Ok("nil".into())
    );
    assert_eq!(state.lua.as_ref().map(|l| l.async_.len()), Some(2));
    assert!(!callback_freed(&state, 1) && !callback_freed(&state, 2));
}

/// About `kib` KiB of Luau that is slow to compile and fast to run: one
/// function it never calls, then `tail`.
fn big_chunk(kib: usize, tail: &str) -> String {
    let mut src = String::from("local function unused()\n  local M = {}\n");
    for i in 0..kib * 1024 / 120 {
        src.push_str(&format!(
            "  function M.f{i}(a, b)\n    if a == b then return a .. '{i}' end\n    return string.format('%s-%d', b, {i})\n  end\n"
        ));
    }
    src.push_str("  return M\nend\n");
    src.push_str(tail);
    src
}

#[test]
fn compiling_a_cold_pack_is_off_the_budget() {
    // A debug build takes ~30 ms to compile 64 KiB. Without the fix the
    // compile alone runs past 50 ms: the init and the module each fail.
    let module = big_chunk(512, "return { ok = 'yes' }\n");
    let t = Instant::now();
    mlua::Lua::new().load(&module).into_function().unwrap();
    let compile = t.elapsed();
    let init = big_chunk(512, "return { mod = require('mod') }\n");
    let dir = folder(&init, &[("mod.luau", &module)]);
    let config = folder("", &[]);
    let mut state = test_state();
    // As a server load does: `require`'s cache starts with the config folder.
    reset_at(&mut state, config.path());
    let t = Instant::now();
    let r = load_pack(&mut state, "big", dir.path(), &serde_json::json!({}));
    assert_eq!(r, Ok(()), "one compile {compile:?}, load {:?}", t.elapsed());
    let r = run_chunk(&mut state, &test_ctx(), "return grofe.plugin('big').mod.ok");
    assert_eq!(r, Ok("yes".into()));
}

#[test]
fn requiring_a_raising_module_in_a_loop_still_ends() {
    // The module's compile is off the budget; its runs are not, so the
    // loop still ends.
    let module = big_chunk(64, "error('raised')\n");
    let dir = folder("", &[("bad.luau", &module)]);
    let mut state = test_state();
    load_init(&mut state, dir.path());
    let t = Instant::now();
    let e = run_chunk(
        &mut state,
        &test_ctx(),
        "while true do pcall(require, 'bad') end",
    )
    .unwrap_err();
    assert!(e.contains("budget"), "{e}");
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
}

#[test]
fn notify_with_no_session_posts_a_server_entry() {
    let mut state = test_state();
    let ctx = test_ctx();
    assert_eq!(
        run_chunk(&mut state, &ctx, "grofe.notify('hi')"),
        Ok("".into())
    );
    let n = &state.notify.items()[0];
    assert_eq!((n.pane, n.msg.as_deref()), (None, Some("hi")));
    // A pane asked for by name must still exist.
    let e = run_chunk(&mut state, &ctx, "grofe.notify('x', {pane = '%9'})").unwrap_err();
    assert!(e.contains("no pane"), "{e}");
}
