//! Helpers the lua_runtime_api tests share.

//! Spec 2026-11-26-issue-agents, Part A, item A1: the runtime API a pack
//! needs (`grofe.notify`, `grofe.json`, `grofe.config`, pack-relative
//! `require`, `grofe.store`, error attribution, manifest `min_api` and
//! `[options]`). One positive and one error test per item, each on a
//! private socket and private dirs (`tests/common`).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::common;
pub(crate) use crate::common::TestServer;

pub(crate) const WAIT: Duration = Duration::from_secs(10);

pub(crate) fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

pub(crate) fn git(dir: &Path, args: &[&str]) {
    let out = common::git(dir).args(args).output().expect("run git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

pub(crate) fn lua(srv: &TestServer, code: &str) -> Result<String, String> {
    srv.cmd(&["lua", code]).map(|s| s.trim().to_string())
}

pub(crate) fn lua_ok(srv: &TestServer, code: &str) -> String {
    lua(srv, code).unwrap_or_else(|e| panic!("lua {code:?}: {e}\nlog:\n{}", srv.log()))
}

pub(crate) fn config_dir(srv: &TestServer) -> PathBuf {
    srv.config_home.join("grofe")
}

/// A started server whose config folder holds `init`.
pub(crate) fn with_init(init: &str) -> TestServer {
    let srv = TestServer::builder().unstarted();
    write(&config_dir(&srv).join("init.luau"), init);
    srv.cmd(&["start-server"]).expect("start-server");
    srv
}

/// One pack: its name and files (the manifest is `grofe-plugin.toml`).
pub(crate) struct Pack<'a> {
    pub name: &'a str,
    pub files: &'a [(&'a str, &'a str)],
    /// Extra lines of the user's `[[pack]]` entry.
    pub entry: &'a str,
}

/// A server with `packs` installed through `pack sync` from local bare
/// repos and loaded at start. `user_init` is the config folder's init.luau.
pub(crate) fn with_packs(packs: &[Pack], user_init: Option<&str>) -> TestServer {
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

pub(crate) fn manifest(name: &str, extra: &str) -> String {
    format!("name = \"{name}\"\nmin_api = 1\n{extra}")
}

pub(crate) fn messages(srv: &TestServer) -> String {
    srv.cmd(&["show-messages"]).unwrap_or_default()
}

// grofe.notify
