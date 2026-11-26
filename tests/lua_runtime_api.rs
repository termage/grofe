//! Spec 2026-11-26-issue-agents, Part A, item A1: the runtime API a pack
//! needs (`grofe.notify`, `grofe.json`, `grofe.config`, pack-relative
//! `require`, `grofe.store`, error attribution, manifest `min_api` and
//! `[options]`). One positive and one error test per item, each on a
//! private socket and private dirs (`tests/common`).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use common::TestServer;

const WAIT: Duration = Duration::from_secs(10);

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        // Temp repos get no hooks from the user's template directory.
        .env("GIT_TEMPLATE_DIR", "")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn lua(srv: &TestServer, code: &str) -> Result<String, String> {
    srv.cmd(&["lua", code]).map(|s| s.trim().to_string())
}

fn lua_ok(srv: &TestServer, code: &str) -> String {
    lua(srv, code).unwrap_or_else(|e| panic!("lua {code:?}: {e}\nlog:\n{}", srv.log()))
}

fn config_dir(srv: &TestServer) -> PathBuf {
    srv.config_home.join("grofe")
}

/// A started server whose config folder holds `init`.
fn with_init(init: &str) -> TestServer {
    let srv = TestServer::builder().unstarted();
    write(&config_dir(&srv).join("init.luau"), init);
    srv.cmd(&["start-server"]).expect("start-server");
    srv
}

/// One pack: its name and files (the manifest is `grofe-plugin.toml`).
struct Pack<'a> {
    name: &'a str,
    files: &'a [(&'a str, &'a str)],
    /// Extra lines of the user's `[[pack]]` entry.
    entry: &'a str,
}

/// A server with `packs` installed through `pack sync` from local bare
/// repos and loaded at start. `user_init` is the config folder's init.luau.
fn with_packs(packs: &[Pack], user_init: Option<&str>) -> TestServer {
    let srv = TestServer::builder().unstarted();
    let root = srv.home.join("repos");
    let mut config = String::new();
    for p in packs {
        let work = root.join(p.name);
        for (f, c) in p.files {
            write(&work.join(f), c);
        }
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "test: pack"]);
        let bare = format!("{}.git", p.name);
        git(&root, &["clone", "-q", "--bare", p.name, &bare]);
        config.push_str(&format!(
            "[[pack]]\nsrc = \"file://{}\"\n{}\n",
            root.join(&bare).display(),
            p.entry
        ));
    }
    write(&config_dir(&srv).join("config.toml"), &config);
    if let Some(init) = user_init {
        write(&config_dir(&srv).join("init.luau"), init);
    }
    let mut cmd = srv.command();
    cmd.args(["pack", "sync"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0");
    let out = common::run_bounded(cmd, common::CMD_TIMEOUT);
    assert!(out.status.success(), "pack sync: {out:?}");
    srv.cmd(&["start-server"]).expect("start-server");
    srv
}

fn manifest(name: &str, extra: &str) -> String {
    format!("name = \"{name}\"\nmin_api = 1\n{extra}")
}

fn messages(srv: &TestServer) -> String {
    srv.cmd(&["show-messages"]).unwrap_or_default()
}

// grofe.notify

#[test]
fn notify_posts_a_tray_entry() {
    let srv = with_init("");
    srv.cmd(&["new-session", "-d", "-s", "main", "cat"])
        .expect("new-session");
    lua_ok(&srv, "grofe.notify('build finished', {level = 'warn'})");
    let list = srv.cmd(&["notify", "list"]).expect("notify list");
    assert!(list.contains("build finished"), "{list}");
    let pane = srv
        .cmd(&["display-message", "-p", "#{pane_id}"])
        .expect("pane id");
    lua_ok(
        &srv,
        &format!("grofe.notify('for a pane', {{pane = '{}'}})", pane.trim()),
    );
    let list = srv.cmd(&["notify", "list"]).expect("notify list");
    assert!(list.contains("for a pane"), "{list}");
}

#[test]
fn notify_rejects_bad_arguments() {
    let srv = TestServer::start();
    let e = lua(&srv, "grofe.notify({})").expect_err("a table is not text");
    assert!(!e.contains("nil value"), "grofe.notify must exist: {e}");
    let e = lua(&srv, "grofe.notify('x', {pane = '%999999'})").expect_err("no such pane");
    assert!(e.contains("pane") || e.contains("999999"), "{e}");
    assert_eq!(srv.cmd(&["notify", "list"]).expect("list").trim(), "");
}

// grofe.json

#[test]
fn json_round_trips_unicode_nested_tables_and_arrays() {
    let srv = TestServer::start();
    let code = "local v = grofe.json.decode(grofe.json.encode({\
        title = 'Grüße 日本語 \\u{1F600}', n = 3, flag = true,\
        list = {10, 20, 30}, nested = {a = {b = {c = 'deep'}}}}))\
        return v.title .. '|' .. v.n .. '|' .. tostring(v.flag) .. '|' .. #v.list .. '|' .. v.list[3] .. '|' .. v.nested.a.b.c";
    assert_eq!(
        lua_ok(&srv, code),
        "Grüße 日本語 \u{1F600}|3|true|3|30|deep"
    );
}

#[test]
fn json_distinguishes_arrays_from_objects() {
    let srv = TestServer::start();
    assert_eq!(
        lua_ok(&srv, "return grofe.json.encode({1, 2, 3})"),
        "[1,2,3]"
    );
    assert_eq!(
        lua_ok(&srv, "return grofe.json.encode({a = 1})"),
        "{\"a\":1}"
    );
    assert_eq!(
        lua_ok(
            &srv,
            "local v = grofe.json.decode('[1,[2],{\"k\":[]}]') return #v .. '|' .. v[2][1] .. '|' .. #v[3].k"
        ),
        "3|2|0"
    );
}

#[test]
fn json_decode_of_invalid_text_raises() {
    let srv = TestServer::start();
    let e = lua(&srv, "return grofe.json.decode('{\"a\": ')").expect_err("must raise");
    assert!(!e.contains("nil value"), "grofe.json must exist: {e}");
    let ok = lua_ok(
        &srv,
        "local ok, err = pcall(grofe.json.decode, 'not json') return tostring(ok)",
    );
    assert_eq!(ok, "false");
}

// grofe.config

fn repo_with_project_config(srv: &TestServer, name: &str, text: &str) -> PathBuf {
    let repo = srv.home.join("proj");
    fs::create_dir_all(repo.join("sub/deeper")).expect("mkdir");
    git(&repo, &["init", "-q", "-b", "main"]);
    write(&repo.join(".grofe").join(format!("{name}.toml")), text);
    repo
}

#[test]
fn config_reads_the_user_file_and_missing_files_are_empty() {
    let srv = TestServer::start();
    write(
        &config_dir(&srv).join("tool.toml"),
        "name = \"user\"\n[opts]\na = 1\n",
    );
    assert_eq!(
        lua_ok(
            &srv,
            "local c = grofe.config('tool') return c.name .. c.opts.a"
        ),
        "user1"
    );
    assert_eq!(
        lua_ok(
            &srv,
            "local n = 0 for _ in pairs(grofe.config('absent')) do n += 1 end return n"
        ),
        "0"
    );
}

#[test]
fn config_project_file_wins_and_merges_nested_tables() {
    let srv = TestServer::start();
    write(
        &config_dir(&srv).join("tool.toml"),
        "name = \"user\"\nkeep = \"u\"\n[opts]\na = 1\nb = 2\n",
    );
    let repo =
        repo_with_project_config(&srv, "tool", "name = \"project\"\n[opts]\nb = 20\nc = 30\n");
    let dir = repo.join("sub/deeper");
    let code = format!(
        "local c = grofe.config('tool', '{}') return c.name .. c.keep .. c.opts.a .. '|' .. c.opts.b .. '|' .. c.opts.c",
        dir.display()
    );
    assert_eq!(lua_ok(&srv, &code), "projectu1|20|30");
    // Outside any repo only the user file applies.
    let code = format!("return grofe.config('tool', '{}').name", srv.home.display());
    assert_eq!(lua_ok(&srv, &code), "user");
}

#[test]
fn config_parse_error_names_file_and_line() {
    let srv = TestServer::start();
    write(
        &config_dir(&srv).join("tool.toml"),
        "ok = 1\n\nbroken = = 2\n",
    );
    let e = lua(&srv, "return grofe.config('tool')").expect_err("must raise");
    assert!(e.contains("tool.toml"), "{e}");
    assert!(e.contains('3'), "names line 3: {e}");

    write(&config_dir(&srv).join("tool.toml"), "ok = 1\n");
    let repo = repo_with_project_config(&srv, "tool", "a = 1\nb = \n");
    let code = format!("return grofe.config('tool', '{}')", repo.display());
    let e = lua(&srv, &code).expect_err("must raise");
    assert!(e.contains(".grofe") && e.contains("tool.toml"), "{e}");
    assert!(e.contains('2'), "names line 2: {e}");
}

// Pack-relative require and pack_dir

#[test]
fn pack_init_requires_a_module_from_its_own_folder() {
    let srv = with_packs(
        &[Pack {
            name: "rq",
            files: &[
                ("grofe-plugin.toml", &manifest("rq", "")),
                (
                    "init.luau",
                    "local m = require('mod')\nreturn { v = m.value, dir = grofe.pack_dir() }\n",
                ),
                ("mod.luau", "return { value = 'from-pack' }\n"),
            ],
            entry: "",
        }],
        None,
    );
    assert_eq!(lua_ok(&srv, "return grofe.plugin('rq').v"), "from-pack");
    let dir = lua_ok(&srv, "return grofe.plugin('rq').dir");
    assert!(Path::new(&dir).join("mod.luau").is_file(), "{dir}");
}

#[test]
fn user_init_still_requires_from_the_config_folder() {
    let srv = with_packs(
        &[Pack {
            name: "rq",
            files: &[
                ("grofe-plugin.toml", &manifest("rq", "")),
                ("init.luau", "return { v = require('mod').value }\n"),
                ("mod.luau", "return { value = 'pack' }\n"),
            ],
            entry: "",
        }],
        Some("grofe.cmd{'set-option','-g','@u', require('mod').value}\n"),
    );
    write(
        &config_dir(&srv).join("mod.luau"),
        "return { value = 'user' }\n",
    );
    srv.cmd(&["reload"]).expect("reload");
    srv.wait_for(
        || {
            srv.cmd(&["show-options", "-gqv", "@u"])
                .is_ok_and(|s| s.trim() == "user")
        },
        WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}; log:\n{}", srv.log()));
    assert_eq!(lua_ok(&srv, "return grofe.plugin('rq').v"), "pack");
}

#[test]
fn pack_require_of_a_missing_module_fails_the_pack_with_its_name() {
    let srv = with_packs(
        &[Pack {
            name: "norq",
            files: &[
                ("grofe-plugin.toml", &manifest("norq", "")),
                ("init.luau", "require('does_not_exist')\n"),
            ],
            entry: "",
        }],
        None,
    );
    let m = messages(&srv);
    assert!(m.contains("norq") && m.contains("does_not_exist"), "{m}");
}

// grofe.store

const COUNTER_PACK: &str = "local n = (grofe.store.get('n') or 0) + 1\n\
    grofe.store.set('n', n)\n\
    grofe.store.set('s', 'text')\n\
    grofe.store.set('t', {id = 'c1', list = {1, 2}})\n\
    grofe.store.set('gone', 'x')\n\
    grofe.store.delete('gone')\n\
    local t = grofe.store.get('t')\n\
    return { n = n, s = grofe.store.get('s'), id = t.id, second = t.list[2], gone = grofe.store.get('gone') == nil }\n";

fn counter_srv() -> TestServer {
    with_packs(
        &[Pack {
            name: "kv",
            files: &[
                ("grofe-plugin.toml", &manifest("kv", "")),
                ("init.luau", COUNTER_PACK),
            ],
            entry: "",
        }],
        None,
    )
}

#[test]
fn store_sets_gets_and_deletes_strings_numbers_and_tables() {
    let srv = counter_srv();
    let code = "local p = grofe.plugin('kv') return p.n .. '|' .. p.s .. '|' .. p.id .. '|' .. p.second .. '|' .. tostring(p.gone)";
    assert_eq!(lua_ok(&srv, code), "1|text|c1|2|true");
}

#[test]
fn store_survives_reload_and_server_restart() {
    let srv = counter_srv();
    assert_eq!(lua_ok(&srv, "return grofe.plugin('kv').n"), "1");
    srv.cmd(&["reload"]).expect("reload");
    assert_eq!(lua_ok(&srv, "return grofe.plugin('kv').n"), "2");
    // `start-server` is not the harness's own child, so `restart` alone would
    // leave this server running.
    let _ = srv.cmd(&["kill-server"]);
    srv.wait_for(|| !srv.is_running(), WAIT)
        .expect("server stops");
    let srv = srv.restart();
    assert_eq!(lua_ok(&srv, "return grofe.plugin('kv').n"), "3");
    assert_eq!(lua_ok(&srv, "return grofe.plugin('kv').id"), "c1");
}

#[test]
fn store_is_private_to_each_pack() {
    let a = "grofe.store.set('secret', 'a-value')\nreturn { seen = grofe.store.get('secret') }\n";
    let b = "return { seen = grofe.store.get('secret') == nil and 'none' or 'leak' }\n";
    let srv = with_packs(
        &[
            Pack {
                name: "pa",
                files: &[("grofe-plugin.toml", &manifest("pa", "")), ("init.luau", a)],
                entry: "",
            },
            Pack {
                name: "pb",
                files: &[("grofe-plugin.toml", &manifest("pb", "")), ("init.luau", b)],
                entry: "",
            },
        ],
        None,
    );
    assert_eq!(lua_ok(&srv, "return grofe.plugin('pa').seen"), "a-value");
    assert_eq!(lua_ok(&srv, "return grofe.plugin('pb').seen"), "none");
}

#[test]
fn store_rejects_values_it_cannot_keep() {
    let srv = with_packs(
        &[Pack {
            name: "kv",
            files: &[
                ("grofe-plugin.toml", &manifest("kv", "")),
                (
                    "init.luau",
                    "local ok = pcall(grofe.store.set, 'f', function() end)\nreturn { ok = tostring(ok) }\n",
                ),
            ],
            entry: "",
        }],
        None,
    );
    assert_eq!(lua_ok(&srv, "return grofe.plugin('kv').ok"), "false");
}

// Error attribution

#[test]
fn a_callback_error_names_the_pack() {
    let srv = with_packs(
        &[Pack {
            name: "errpack",
            files: &[
                ("grofe-plugin.toml", &manifest("errpack", "")),
                (
                    "init.luau",
                    "grofe.every(50, function() error('tick-boom') end)\n",
                ),
            ],
            entry: "",
        }],
        None,
    );
    srv.wait_for(|| messages(&srv).contains("tick-boom"), WAIT)
        .unwrap_or_else(|e| panic!("{e}; messages:\n{}\nlog:\n{}", messages(&srv), srv.log()));
    let m = messages(&srv);
    assert!(
        m.lines()
            .any(|l| l.contains("tick-boom") && l.contains("pack errpack")),
        "{m}"
    );
}

#[test]
fn a_pack_load_error_appears_in_show_messages() {
    let srv = with_packs(
        &[Pack {
            name: "badload",
            files: &[
                ("grofe-plugin.toml", &manifest("badload", "")),
                ("init.luau", "error('load-boom')\n"),
            ],
            entry: "",
        }],
        None,
    );
    let m = messages(&srv);
    assert!(m.contains("badload") && m.contains("load-boom"), "{m}");
}

#[test]
fn a_pack_that_fails_halfway_leaves_nothing_registered() {
    let init = "grofe.bind('prefix', 'F9', function() end, {desc = 'half-bind'})\n\
        grofe.every(50, function() grofe.cmd{'set-option','-g','@half','ticked'} end)\n\
        grofe.on('window-created', function() grofe.cmd{'set-option','-g','@sub','fired'} end)\n\
        error('halfway')\n";
    let srv = with_packs(
        &[Pack {
            name: "half",
            files: &[
                ("grofe-plugin.toml", &manifest("half", "")),
                ("init.luau", init),
            ],
            entry: "",
        }],
        None,
    );
    srv.cmd(&["new-session", "-d", "-s", "main", "cat"])
        .expect("new-session");
    assert!(messages(&srv).contains("halfway"), "{}", messages(&srv));
    let keys = srv.cmd(&["list-keys", "-T", "prefix"]).expect("list-keys");
    assert!(!keys.contains("half-bind"), "bind left behind:\n{keys}");
    srv.cmd(&["new-window", "-d"]).expect("new-window");
    std::thread::sleep(Duration::from_millis(500));
    let opt = |n: &str| srv.cmd(&["show-options", "-gqv", n]).unwrap_or_default();
    assert_eq!(opt("@half").trim(), "", "timer left behind");
    assert_eq!(opt("@sub").trim(), "", "subscription left behind");
}

// Manifest: min_api and [options]

#[test]
fn min_api_at_or_below_the_grofe_api_loads() {
    let srv = with_packs(
        &[Pack {
            name: "newer",
            files: &[
                ("grofe-plugin.toml", "name = \"newer\"\nmin_api = 1\n"),
                ("init.luau", "return { ok = 'yes' }\n"),
            ],
            entry: "",
        }],
        None,
    );
    assert_eq!(lua_ok(&srv, "return grofe.plugin('newer').ok"), "yes");
}

#[test]
fn min_api_above_the_grofe_api_is_refused_with_a_message() {
    let srv = with_packs(
        &[Pack {
            name: "toonew",
            files: &[
                ("grofe-plugin.toml", "name = \"toonew\"\nmin_api = 9999\n"),
                ("init.luau", "return { ok = 'yes' }\n"),
            ],
            entry: "",
        }],
        None,
    );
    assert_eq!(
        lua_ok(&srv, "return tostring(grofe.plugin('toonew'))"),
        "nil"
    );
    let m = messages(&srv);
    assert!(m.contains("toonew") && m.contains("9999"), "{m}");
}

const OPTIONS_INIT: &str = "local o = grofe.options()\n\
    return { greeting = o.greeting, retries = o.retries, nested = o.deep.flag }\n";

fn options_srv(entry: &str) -> TestServer {
    with_packs(
        &[Pack {
            name: "opt",
            files: &[
                (
                    "grofe-plugin.toml",
                    &manifest(
                        "opt",
                        "[options]\ngreeting = \"hello\"\nretries = 3\n[options.deep]\nflag = \"d\"\n",
                    ),
                ),
                ("init.luau", OPTIONS_INIT),
            ],
            entry,
        }],
        None,
    )
}

#[test]
fn options_defaults_are_readable_from_lua() {
    let srv = options_srv("");
    let code =
        "local p = grofe.plugin('opt') return p.greeting .. '|' .. p.retries .. '|' .. p.nested";
    assert_eq!(lua_ok(&srv, code), "hello|3|d");
}

#[test]
fn options_are_overridden_by_the_users_pack_entry() {
    let srv = options_srv("options = { greeting = \"hoi\" }\n");
    let code =
        "local p = grofe.plugin('opt') return p.greeting .. '|' .. p.retries .. '|' .. p.nested";
    assert_eq!(lua_ok(&srv, code), "hoi|3|d");
}

#[test]
fn options_called_outside_a_pack_raises() {
    let srv = TestServer::start();
    let e = lua(&srv, "return grofe.options()").expect_err("no pack, no options");
    assert!(!e.contains("nil value"), "grofe.options must exist: {e}");
}
