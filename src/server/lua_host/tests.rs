use super::*;
use crate::server::{test_ctx, test_state};
use std::time::Instant;

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
    let t = Instant::now();
    let e = run_chunk(&mut state, &test_ctx(), "while true do end").unwrap_err();
    assert!(e.contains("budget"), "{e}");
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "{:?}",
        t.elapsed()
    );
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
    assert!(run_chunk(&mut state, &ctx, "grofe.cmd{'reload'}").is_ok());
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
    load_pack(&mut state, "p", dir.path()).unwrap();
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
    let t = Instant::now();
    let e = require(&mut state, "spin").unwrap_err();
    assert!(e.contains("budget"), "{e}");
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "{:?}",
        t.elapsed()
    );
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

/// A9: a pack's callback keeps mlua's `require` after init set the folder,
/// while a callback init defined loads from the folder.
#[test]
fn a_pack_callback_keeps_the_old_require() {
    let pack = tempfile::tempdir().unwrap();
    std::fs::write(
        pack.path().join("init.luau"),
        "grofe.bind('prefix','p',function()\n\
           local ok, e = pcall(require, 'm')\n\
           grofe.cmd{'set-option','-g','@pack',tostring(ok) .. ' ' .. tostring(e)}\n\
         end)",
    )
    .unwrap();
    let dir = folder(
        "grofe.bind('prefix','g',function() grofe.cmd{'set-option','-g','@init',require('m')} end)",
        &[("m.luau", "return 'folder'")],
    );
    let mut state = test_state();
    load_pack(&mut state, "p", pack.path()).unwrap();
    load_init(&mut state, dir.path());
    call(&mut state, 1, Payload::None);
    call(&mut state, 2, Payload::None);
    let got = global(&mut state, "@pack");
    assert!(got.starts_with("false") && !got.contains("folder"), "{got}");
    assert!(
        !got.contains("not found"),
        "pack got the folder require: {got}"
    );
    assert_eq!(
        global(&mut state, "@init"),
        "folder",
        "{}",
        messages(&state)
    );
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
