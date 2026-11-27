//! notify, json and config.

use std::fs;
use std::path::PathBuf;

use crate::support::*;

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
fn notify_rejects_an_unknown_level_and_posts_with_no_session() {
    let srv = TestServer::start();
    let e = lua(&srv, "grofe.notify('x', {level = 'loud'})").expect_err("bad level");
    assert!(e.contains("level"), "{e}");
    assert_eq!(srv.cmd(&["notify", "list"]).expect("list").trim(), "");
    // The tray is server-wide: with no session the entry is the server's.
    lua_ok(&srv, "grofe.notify('server-wide')");
    let list = srv.cmd(&["notify", "list"]).expect("list");
    assert!(list.contains("server-wide"), "{list}");
}

#[test]
fn config_refuses_odd_and_reserved_names() {
    let srv = TestServer::start();
    for name in ["../x", "a/b", "config", "pack-lock", ""] {
        let e = lua(&srv, &format!("return grofe.config('{name}')")).expect_err(name);
        assert!(!e.contains("nil value"), "{name}: {e}");
    }
}

#[test]
fn json_holes_become_null_and_round_trip() {
    let srv = TestServer::start();
    assert_eq!(
        lua_ok(&srv, "return grofe.json.encode({1, nil, 3})"),
        "[1,null,3]"
    );
    assert_eq!(
        lua_ok(
            &srv,
            "return grofe.json.encode(grofe.json.decode('[1,null,3]'))"
        ),
        "[1,null,3]"
    );
}
