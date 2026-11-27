//! `grofe pack sync` from a local bare repo, then `reload` loads the plugin.

mod common;

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use common::{CMD_TIMEOUT, CmdOutput, TestServer, run_bounded};

fn git(dir: &Path, args: &[&str]) {
    let out = common::git(dir).args(args).output().expect("run git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A server (not started) with one local pack `p` whose `init.luau` is
/// `init`, configured and ready for `pack sync`.
fn server_with_pack(init: &str) -> TestServer {
    let srv = TestServer::builder().unstarted();
    let root = srv.home.join("repos");
    let work = root.join("work");
    let bare = root.join("p.git");
    fs::create_dir_all(&work).expect("mkdir work");
    fs::write(work.join("grofe-plugin.toml"), "name = \"p\"\napi = 1\n").expect("write manifest");
    fs::write(work.join("init.luau"), init).expect("write init");
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["add", "."]);
    // No hook of the user's runs on the temp repo.
    git(&work, &["commit", "-q", "--no-verify", "-m", "p"]);
    git(&root, &["clone", "-q", "--bare", "work", "p.git"]);

    let config = srv.config_home.join("grofe");
    fs::create_dir_all(&config).expect("mkdir config");
    fs::write(
        config.join("config.toml"),
        format!("[[pack]]\nsrc = \"file://{}\"\n", bare.display()),
    )
    .expect("write config");
    srv
}

/// Every grofe a test runs sees the same private data dir, whatever the
/// host's own XDG_DATA_HOME says.
fn run(srv: &TestServer, args: &[&str]) -> CmdOutput {
    let mut cmd = srv.command();
    cmd.args(args)
        .env("XDG_DATA_HOME", srv.home.join("data"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0");
    run_bounded(cmd, CMD_TIMEOUT)
}

#[test]
fn pack_sync_then_reload_loads_plugin() {
    let srv = server_with_pack("return { x = 1 }\n");
    let config = srv.config_home.join("grofe");

    let out = run(&srv, &["pack", "sync"]);
    assert!(
        out.status.success(),
        "pack sync: {} {}",
        out.stdout,
        out.stderr
    );
    assert!(config.join("pack-lock.toml").exists(), "no lock written");

    let out = run(&srv, &["start-server"]);
    assert!(out.status.success(), "start-server: {}", out.stderr);
    let out = run(&srv, &["reload"]);
    assert!(out.status.success(), "reload: {}", out.stderr);
    let out = run(&srv, &["lua", "return grofe.plugin(\"p\").x"]);
    assert!(
        out.status.success() && out.stdout.trim() == "1",
        "lua: {:?} {:?}\nlog:\n{}",
        out.stdout,
        out.stderr,
        srv.log()
    );
}

/// A pack whose top-level code runs a slow `run-shell` neither holds up the
/// server's start nor lends its output to the first request's reply.
#[test]
fn pack_run_shell_at_load_does_not_hijack_the_first_reply() {
    let srv = server_with_pack("grofe.cmd{'run-shell','echo from-pack; sleep 3'}\nreturn {}\n");
    assert!(run(&srv, &["pack", "sync"]).status.success());
    let t = Instant::now();
    let out = run(
        &srv,
        &["new-session", "-d", "-s", "main", "-P", "-F", "#{pane_id}"],
    );
    assert!(out.status.success(), "{}", out.stderr);
    assert!(
        out.stdout.trim().starts_with('%') && !out.stdout.contains("from-pack"),
        "got {:?}",
        out.stdout
    );
    assert!(t.elapsed() < Duration::from_secs(2), "start waited");
}
