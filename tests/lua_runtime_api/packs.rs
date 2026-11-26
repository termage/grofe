//! Packs: require, store, options, manifest, errors and keys.

use std::path::Path;
use std::time::Duration;

use crate::support::*;

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

#[test]
fn a_pack_function_called_from_user_init_acts_as_its_pack() {
    let init = "return { setup = function()\n\
        grofe.store.set('k', 'v')\n\
        grofe.every(50, function() error('late-boom') end)\n\
        return grofe.store.get('k') .. '|' .. tostring(#grofe.pack_dir() > 0)\n\
        end }\n";
    let srv = with_packs(
        &[Pack {
            name: "late",
            files: &[
                ("grofe-plugin.toml", &manifest("late", "")),
                ("init.luau", init),
            ],
            entry: "",
        }],
        Some("grofe.cmd{'set-option','-g','@got', grofe.plugin('late').setup()}\n"),
    );
    let got = |n: &str| srv.cmd(&["show-options", "-gqv", n]).unwrap_or_default();
    assert_eq!(got("@got").trim(), "v|true", "{}", messages(&srv));
    srv.wait_for(|| messages(&srv).contains("late-boom"), WAIT)
        .unwrap_or_else(|e| panic!("{e}; messages:\n{}", messages(&srv)));
    let m = messages(&srv);
    assert!(
        m.lines()
            .any(|l| l.contains("late-boom") && l.contains("pack late")),
        "{m}"
    );
}

#[test]
fn a_pack_callback_can_use_the_store() {
    let init = "grofe.store.set('x', 'kept')\n\
        grofe.every(50, function() grofe.cmd{'set-option','-g','@cb', grofe.store.get('x')} end)\n";
    let srv = with_packs(
        &[Pack {
            name: "cbstore",
            files: &[
                ("grofe-plugin.toml", &manifest("cbstore", "")),
                ("init.luau", init),
            ],
            entry: "",
        }],
        None,
    );
    srv.wait_for(
        || {
            srv.cmd(&["show-options", "-gqv", "@cb"])
                .is_ok_and(|s| s.trim() == "kept")
        },
        WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}; messages:\n{}", messages(&srv)));
}

#[test]
fn pack_only_functions_raise_outside_a_pack() {
    let srv = TestServer::start();
    for code in [
        "return grofe.pack_dir()",
        "return grofe.store.get('k')",
        "grofe.store.set('k', 1)",
        "grofe.store.delete('k')",
    ] {
        let e = lua(&srv, code).expect_err(code);
        assert!(e.contains("outside a pack"), "{code}: {e}");
    }
}

#[test]
fn options_from_the_entry_merge_into_a_nested_manifest_table_key_by_key() {
    let srv = with_packs(
        &[Pack {
            name: "nest",
            files: &[
                (
                    "grofe-plugin.toml",
                    &manifest("nest", "[options.deep]\nflag = \"d\"\nkeep = \"k\"\n"),
                ),
                (
                    "init.luau",
                    "local o = grofe.options().deep\nreturn { v = o.flag .. '|' .. o.keep .. '|' .. o.added }\n",
                ),
            ],
            entry: "options = { deep = { flag = \"over\", added = \"a\" } }\n",
        }],
        None,
    );
    assert_eq!(lua_ok(&srv, "return grofe.plugin('nest').v"), "over|k|a");
}

#[test]
fn a_failed_pack_gives_back_the_key_it_replaced() {
    let init =
        "grofe.bind('prefix', 'c', function() end, {desc = 'stolen-key'})\nerror('after-bind')\n";
    let srv = with_packs(
        &[Pack {
            name: "thief",
            files: &[
                ("grofe-plugin.toml", &manifest("thief", "")),
                ("init.luau", init),
            ],
            entry: "",
        }],
        None,
    );
    let keys = srv.cmd(&["list-keys", "-T", "prefix"]).expect("list-keys");
    assert!(!keys.contains("stolen-key"), "{keys}");
    assert!(
        keys.lines()
            .any(|l| l.split_whitespace().any(|w| w == "c") && l.contains("new-window")),
        "the default c binding is back:\n{keys}"
    );
}

#[test]
fn cancelling_two_binds_out_of_order_ends_at_the_default_key() {
    let srv = TestServer::start();
    lua_ok(
        &srv,
        "local a = grofe.bind('prefix', 'c', function() end, {desc = 'first'})\n\
         local b = grofe.bind('prefix', 'c', function() end, {desc = 'second'})\n\
         grofe.cancel(a)\n\
         grofe.cancel(b)",
    );
    let keys = srv.cmd(&["list-keys", "-T", "prefix"]).expect("list-keys");
    assert!(!keys.contains("lua-call"), "{keys}");
    assert!(
        keys.lines()
            .any(|l| l.split_whitespace().any(|w| w == "c") && l.contains("new-window")),
        "the default c binding is back:\n{keys}"
    );
}
