//! Lua `require` from the config folder (spec 2026-11-25-config-dir-and-shrink,
//! A6, test t6). The unit tests for A7 to A9 live in `src/server/lua_host.rs`.

mod common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use common::{CMD_TIMEOUT, CmdOutput, TestServer, run_bounded};

const WAIT: Duration = Duration::from_secs(10);

fn run(srv: &TestServer, args: &[&str]) -> CmdOutput {
    let mut cmd = srv.command();
    cmd.env_remove("GROFE_CONFIG")
        .env_remove("GROFE_CONFIG_DIR");
    cmd.args(args);
    run_bounded(cmd, CMD_TIMEOUT)
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

fn show(srv: &TestServer, name: &str) -> String {
    run(srv, &["show-options", "-gqv", name])
        .stdout
        .trim()
        .to_string()
}

#[test]
fn t6_init_requires_a_keymaps_module_and_a_dotted_ui_module() {
    let srv = TestServer::unstarted();
    let dir = srv.home.join("cust");
    write(
        &dir.join("init.luau"),
        "local km = require('keymaps')\n\
         require('ui.status')\n\
         grofe.cmd{'set-option','-g','@km','' .. km.name}\n",
    );
    write(
        &dir.join("keymaps.luau"),
        "grofe.bind('prefix','g',function() end,{desc='from keymaps'})\nreturn { name = 'km' }\n",
    );
    write(
        &dir.join("ui").join("status.luau"),
        "grofe.cmd{'set-option','-g','@ui','status'}\n",
    );
    let out = run(
        &srv,
        &[
            "--config-dir",
            dir.to_str().expect("utf8"),
            "new-session",
            "-d",
        ],
    );
    assert!(out.status.success(), "{out:?}");
    srv.wait_for(|| show(&srv, "@ui") == "status", WAIT)
        .unwrap_or_else(|e| panic!("{e}; log:\n{}", srv.log()));
    assert_eq!(show(&srv, "@km"), "km");
    let keys = run(&srv, &["list-keys", "-T", "prefix"]).stdout;
    assert!(keys.contains("from keymaps"), "{keys}");
}

#[test]
fn t6_the_book_layout_loads_end_to_end() {
    let srv = TestServer::unstarted();
    let dir = srv.home.join("cust");
    write(
        &dir.join("config.toml"),
        "include = [\"keymaps.toml\", \"theme.toml\"]\n",
    );
    write(
        &dir.join("keymaps.toml"),
        "[keys.prefix]\nF9 = \"display-message hi\"\n",
    );
    write(&dir.join("theme.toml"), "status.left = \"T\"\n");
    write(
        &dir.join("init.luau"),
        "require('keymaps')\nrequire('ui.status')\n",
    );
    write(
        &dir.join("keymaps.luau"),
        "grofe.bind('prefix','g',function() end,{desc='lua key'})\n",
    );
    write(
        &dir.join("ui").join("status.luau"),
        "grofe.cmd{'set-option','-g','@ui','ok'}\n",
    );
    let out = run(
        &srv,
        &[
            "--config-dir",
            dir.to_str().expect("utf8"),
            "new-session",
            "-d",
        ],
    );
    assert!(out.status.success(), "{out:?}");
    srv.wait_for(|| show(&srv, "@ui") == "ok", WAIT)
        .unwrap_or_else(|e| panic!("{e}; log:\n{}", srv.log()));
    assert_eq!(show(&srv, "status-left"), "T");
    let keys = run(&srv, &["list-keys", "-T", "prefix"]).stdout;
    assert!(keys.contains("lua key") && keys.contains("F9"), "{keys}");
}
