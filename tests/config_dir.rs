//! `--config-dir` and `GROFE_CONFIG_DIR` (spec 2026-11-25-config-dir-and-shrink,
//! A1 to A4, tests t1 to t3). Every test uses a private `-S` socket and temp
//! HOME/XDG dirs.

mod common;
#[allow(dead_code)]
#[path = "acceptance/harness.rs"]
mod harness;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{CMD_TIMEOUT, CmdOutput, TestServer, run_bounded, try_wait_until};
use harness::Harness;

const WAIT: Duration = Duration::from_secs(10);

/// Runs the CLI with `GROFE_CONFIG`, `GROFE_CONFIG_DIR` set or cleared, then
/// `args`.
fn run_env(srv: &TestServer, config: Option<&Path>, dir: Option<&str>, args: &[&str]) -> CmdOutput {
    let mut cmd = srv.command();
    cmd.env_remove("GROFE_CONFIG")
        .env_remove("GROFE_CONFIG_DIR");
    if let Some(p) = config {
        cmd.env("GROFE_CONFIG", p);
    }
    if let Some(d) = dir {
        cmd.env("GROFE_CONFIG_DIR", d);
    }
    cmd.args(args);
    run_bounded(cmd, CMD_TIMEOUT)
}

fn run(srv: &TestServer, args: &[&str]) -> CmdOutput {
    run_env(srv, None, None, args)
}

fn fmt(srv: &TestServer, format: &str) -> String {
    let out = run(srv, &["display-message", "-p", format]);
    assert!(out.status.success(), "display failed: {out:?}");
    out.stdout.trim_end().to_string()
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

fn folder(srv: &TestServer, name: &str) -> PathBuf {
    srv.home.join(name)
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf8")
}

/// Starts the server with `--config-dir <dir>` and asserts it came up.
fn start_with_dir(srv: &TestServer, dir: &Path) {
    let out = run(srv, &["--config-dir", s(dir), "new-session", "-d"]);
    assert!(out.status.success(), "{out:?}");
}

/// A folder holding the two files of the spec's t1.
fn t1_folder(srv: &TestServer, name: &str) -> PathBuf {
    let dir = folder(srv, name);
    write(&dir.join("config.toml"), "[status]\nleft = \"A\"\n");
    write(
        &dir.join("conf.d").join("x.toml"),
        "[status]\nright = \"B\"\n",
    );
    dir
}

#[test]
fn t1_config_dir_flag_loads_config_toml_and_conf_d() {
    let srv = TestServer::unstarted();
    let dir = t1_folder(&srv, "cust");
    start_with_dir(&srv, &dir);
    assert_eq!(fmt(&srv, "#{status-left}"), "A");
    assert_eq!(fmt(&srv, "#{status-right}"), "B");
}

#[test]
fn t1_config_dir_env_loads_config_toml_and_conf_d() {
    let srv = TestServer::unstarted();
    let dir = t1_folder(&srv, "cust");
    let out = run_env(&srv, None, Some(s(&dir)), &["new-session", "-d"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fmt(&srv, "#{status-left}"), "A");
    assert_eq!(fmt(&srv, "#{status-right}"), "B");
}

#[test]
fn t1_the_layers_apply_in_order_and_the_folder_replaces_the_xdg_one() {
    let srv = TestServer::unstarted();
    let dir = folder(&srv, "cust");
    write(&dir.join("config.toml"), "status.left = \"config\"\n");
    write(
        &dir.join("conf.d").join("x.toml"),
        "status.left = \"confd\"\n",
    );
    write(&dir.join("user.toml"), "status.left = \"user\"\n");
    write(&dir.join("settings.toml"), "status.left = \"settings\"\n");
    write(
        &srv.config_home.join("grofe").join("user.toml"),
        "status.left = \"WRONG\"\n",
    );
    start_with_dir(&srv, &dir);
    assert_eq!(fmt(&srv, "#{status-left}"), "settings");
    fs::remove_file(dir.join("settings.toml")).expect("rm");
    assert!(run(&srv, &["reload"]).status.success());
    assert_eq!(fmt(&srv, "#{status-left}"), "user");
}

#[test]
fn t1_init_luau_comes_from_the_folder_and_init_lua_is_the_fallback() {
    let srv = TestServer::unstarted();
    let dir = folder(&srv, "cust");
    write(&dir.join("config.toml"), "");
    let init = |v: &str| {
        format!("grofe.every(100, function() grofe.cmd{{'set-option','-g','@from','{v}'}} end)\n")
    };
    write(&dir.join("init.lua"), &init("lua"));
    write(
        &srv.config_home.join("grofe").join("init.luau"),
        &init("wrong"),
    );
    start_with_dir(&srv, &dir);
    srv.wait_for(
        || {
            run(&srv, &["display-message", "-p", "#{@from}"])
                .stdout
                .trim()
                == "lua"
        },
        WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn t1_layouts_come_from_the_folder() {
    let srv = TestServer::unstarted();
    let dir = folder(&srv, "cust");
    write(&dir.join("layouts").join("xlayout.toml"), "");
    let out = run(&srv, &["--config-dir", s(&dir), "layout", "list"]);
    assert!(out.stdout.contains("xlayout"), "{out:?}");
}

#[test]
fn t1_a_folder_with_spaces_and_unicode_and_an_empty_conf_d() {
    let srv = TestServer::unstarted();
    let dir = folder(&srv, "Kønfig dir");
    write(&dir.join("config.toml"), "status.left = \"ø\"\n");
    fs::create_dir_all(dir.join("conf.d")).expect("mkdir");
    start_with_dir(&srv, &dir);
    assert_eq!(fmt(&srv, "#{status-left}"), "ø");
}

#[test]
fn t1_a_folder_that_is_a_symlink_loads() {
    let srv = TestServer::unstarted();
    let real = t1_folder(&srv, "real");
    let link = folder(&srv, "link");
    symlink(&real, &link).expect("symlink");
    start_with_dir(&srv, &link);
    assert_eq!(fmt(&srv, "#{status-left}"), "A");
}

#[test]
fn t2_config_file_with_config_dir_reads_the_file_and_the_other_layers_from_the_folder() {
    let srv = TestServer::unstarted();
    let f = folder(&srv, "f").join("f.toml");
    write(&f, "status.left = \"F\"\n");
    let dir = folder(&srv, "d");
    write(&dir.join("config.toml"), "status.left = \"WRONG\"\n");
    write(&dir.join("user.toml"), "status.right = \"U\"\n");
    let out = run(
        &srv,
        &[
            "--config",
            s(&f),
            "--config-dir",
            s(&dir),
            "new-session",
            "-d",
        ],
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fmt(&srv, "#{status-left}"), "F");
    assert_eq!(fmt(&srv, "#{status-right}"), "U");
}

#[test]
fn t2_env_file_with_env_dir_reads_the_file_and_the_other_layers_from_the_folder() {
    let srv = TestServer::unstarted();
    let f = folder(&srv, "f").join("f.toml");
    write(&f, "status.left = \"F\"\n");
    write(
        &folder(&srv, "f").join("user.toml"),
        "status.right = \"WRONG\"\n",
    );
    let dir = folder(&srv, "d");
    write(&dir.join("user.toml"), "status.right = \"U\"\n");
    let out = run_env(&srv, Some(&f), Some(s(&dir)), &["new-session", "-d"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fmt(&srv, "#{status-left}"), "F");
    assert_eq!(fmt(&srv, "#{status-right}"), "U");
}

#[test]
fn t2_the_flag_beats_the_env_var_for_the_folder() {
    let srv = TestServer::unstarted();
    let (a, b) = (folder(&srv, "a"), folder(&srv, "b"));
    write(&a.join("config.toml"), "status.left = \"A\"\n");
    write(&b.join("config.toml"), "status.left = \"B\"\n");
    let out = run_env(
        &srv,
        None,
        Some(s(&b)),
        &["--config-dir", s(&a), "new-session", "-d"],
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fmt(&srv, "#{status-left}"), "A");
}

#[test]
fn t2_only_config_keeps_the_files_parent_as_the_folder() {
    let srv = TestServer::unstarted();
    let f = folder(&srv, "f").join("f.toml");
    write(&f, "");
    write(
        &folder(&srv, "f").join("user.toml"),
        "status.right = \"P\"\n",
    );
    write(
        &srv.config_home.join("grofe").join("user.toml"),
        "status.right = \"WRONG\"\n",
    );
    let out = run(&srv, &["--config", s(&f), "new-session", "-d"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fmt(&srv, "#{status-right}"), "P");
}

#[test]
fn t2_an_empty_config_dir_env_var_is_ignored_with_config() {
    let srv = TestServer::unstarted();
    let f = folder(&srv, "f").join("f.toml");
    write(&f, "status.left = \"F\"\n");
    write(
        &folder(&srv, "f").join("user.toml"),
        "status.right = \"P\"\n",
    );
    let out = run_env(&srv, Some(&f), Some(""), &["new-session", "-d"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fmt(&srv, "#{status-left}"), "F");
    assert_eq!(fmt(&srv, "#{status-right}"), "P");
}

#[test]
fn t2_an_empty_config_dir_env_var_is_ignored_without_config() {
    let srv = TestServer::unstarted();
    write(
        &srv.config_home.join("grofe").join("config.toml"),
        "status.left = \"X\"\n",
    );
    let out = run_env(&srv, None, Some(""), &["new-session", "-d"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fmt(&srv, "#{status-left}"), "X");
}

#[test]
fn t2_an_empty_config_env_var_is_ignored_with_config_dir() {
    let srv = TestServer::unstarted();
    let dir = t1_folder(&srv, "cust");
    let mut cmd = srv.command();
    cmd.env("GROFE_CONFIG", "")
        .env_remove("GROFE_CONFIG_DIR")
        .args(["--config-dir", s(&dir), "new-session", "-d"]);
    let out = run_bounded(cmd, CMD_TIMEOUT);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fmt(&srv, "#{status-left}"), "A");
}

#[test]
fn t3_reload_reads_the_folders_settings_toml() {
    let srv = TestServer::unstarted();
    let dir = t1_folder(&srv, "cust");
    start_with_dir(&srv, &dir);
    write(&dir.join("settings.toml"), "status.left = \"S\"\n");
    // The reload client carries no folder: the server remembers it.
    assert!(run(&srv, &["reload"]).status.success());
    assert_eq!(fmt(&srv, "#{status-left}"), "S");
}

#[test]
fn t3_settings_menu_apply_writes_the_folders_settings_toml() {
    let srv = TestServer::unstarted();
    let dir = folder(&srv, "cust");
    write(&dir.join("config.toml"), "");
    let out = run(
        &srv,
        &[
            "--config-dir",
            s(&dir),
            "new-session",
            "-d",
            "-s",
            "s",
            "-x",
            "100",
            "-y",
            "30",
            "cat",
        ],
    );
    assert!(out.status.success(), "{out:?}");
    let mut h = Harness::spawn_attach_with_env(srv.command(), "s", &[], 100, 30);
    h.send("\x02C");
    h.wait_until("menu open", WAIT, |t| t.contains("Appearance"));
    h.send("mouse");
    h.send("\r");
    h.send("\x13");
    h.wait_until("diff", WAIT, |t| t.contains("settings.toml"));
    h.send("y");
    let file = dir.join("settings.toml");
    try_wait_until(
        || fs::read_to_string(&file).is_ok_and(|t| t.contains("mouse")),
        WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}\n{}", h.text()));
    assert!(
        !srv.config_home.join("grofe").join("settings.toml").exists(),
        "wrote the XDG folder instead"
    );
}

#[test]
fn t3_a_client_with_another_folder_gets_the_notice() {
    let srv = TestServer::unstarted();
    let (a, b) = (t1_folder(&srv, "a"), t1_folder(&srv, "b"));
    start_with_dir(&srv, &a);
    let out = run(&srv, &["--config-dir", s(&b), "list-sessions"]);
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.contains("grofe: server uses"), "{out:?}");
    let out = run(&srv, &["--config-dir", s(&a), "list-sessions"]);
    assert!(!out.stderr.contains("server uses"), "{out:?}");
}

#[test]
fn a4_a_config_dir_that_is_a_file_is_refused_naming_the_path() {
    let srv = TestServer::unstarted();
    let file = folder(&srv, "afile");
    write(&file, "x");
    let out = run(&srv, &["--config-dir", s(&file), "new-session", "-d"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stderr.contains(s(&file)), "{out:?}");
    assert!(!srv.is_running());
}

#[test]
fn a4_a_missing_config_dir_starts_with_defaults() {
    let srv = TestServer::unstarted();
    let dir = folder(&srv, "missing");
    start_with_dir(&srv, &dir);
    assert!(srv.is_running());
}
