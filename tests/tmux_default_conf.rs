//! Started as `tmux` with no explicit config, grofe reads the default tmux
//! config files like tmux does (spec 2026-11-25-config-dir-and-shrink, A11 and
//! A12, tests t10 and t11). HOME is a temp folder, `XDG_CONFIG_HOME` is unset
//! unless a test sets it. The "HOME unset, passwd home" edge is not tested:
//! the passwd home is the real user's, which a test must never read.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

use common::{CMD_TIMEOUT, CmdOutput, TestServer, run_bounded};

/// A `tmux` identity call: `XDG_CONFIG_HOME` is `xdg` or unset.
fn tmux_with(srv: &TestServer, xdg: Option<&Path>, args: &[&str]) -> CmdOutput {
    tmux_env(srv, xdg, &[], args)
}

/// [`tmux_with`] plus extra environment variables.
fn tmux_env(
    srv: &TestServer,
    xdg: Option<&Path>,
    envs: &[(&str, &Path)],
    args: &[&str],
) -> CmdOutput {
    let root = srv.home.parent().expect("root").to_path_buf();
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let link: PathBuf = bin.join("tmux");
    if !link.exists() {
        symlink(env!("CARGO_BIN_EXE_grofe"), &link).unwrap();
    }
    let tmpdir = root.join("tmux-tmpdir");
    fs::create_dir_all(&tmpdir).unwrap();
    let mut cmd = Command::new(link);
    cmd.current_dir(&srv.home)
        .env_remove("GROFE")
        .env_remove("GROFE_PANE")
        .env_remove("GROFE_CONFIG")
        .env_remove("GROFE_CONFIG_DIR")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", &srv.home)
        .env("XDG_STATE_HOME", &srv.state_home)
        .env("XDG_RUNTIME_DIR", &srv.runtime_dir)
        .env("TMUX_TMPDIR", tmpdir)
        .env("GROFE_RESTORE_ON_START", "0")
        .env("GROFE_REMAIN_ON_EXIT", "1")
        .env("GROFE_NOTIFY_DESKTOP", "off")
        .arg("-S")
        .arg(&srv.socket)
        .args(args);
    if let Some(x) = xdg {
        cmd.env("XDG_CONFIG_HOME", x);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    run_bounded(cmd, CMD_TIMEOUT)
}

fn tmux(srv: &TestServer, args: &[&str]) -> CmdOutput {
    tmux_with(srv, None, args)
}

fn ok(srv: &TestServer, xdg: Option<&Path>, args: &[&str]) -> String {
    let out = tmux_with(srv, xdg, args);
    assert!(out.status.success(), "{args:?}: {}", out.stderr);
    out.stdout
}

fn option(srv: &TestServer, name: &str) -> String {
    ok(srv, None, &["show-options", "-gqv", name])
        .trim()
        .to_string()
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

fn home_conf(srv: &TestServer) -> PathBuf {
    srv.home.join(".tmux.conf")
}

fn xdg_conf(srv: &TestServer) -> PathBuf {
    srv.home.join(".config").join("tmux").join("tmux.conf")
}

fn start(srv: &TestServer) {
    ok(srv, None, &["new-session", "-d"]);
}

fn kill(srv: &TestServer) {
    let _ = tmux(srv, &["kill-server"]);
}

/// The server always reads the real `/etc/tmux.conf`, so the t10 order
/// tests only hold on a host without one.
fn host_has_etc_tmux_conf() -> bool {
    let present = Path::new("/etc/tmux.conf").exists();
    if present {
        eprintln!("skip: /etc/tmux.conf exists on this host");
    }
    present
}

#[test]
fn t10_home_tmux_conf_is_read_without_f() {
    if host_has_etc_tmux_conf() {
        return;
    }
    let srv = TestServer::unstarted();
    write(&home_conf(&srv), "set -g status-left X\n");
    start(&srv);
    assert_eq!(option(&srv, "status-left"), "X");
    kill(&srv);
}

#[test]
fn t10_both_files_load_in_order_so_the_xdg_one_wins() {
    if host_has_etc_tmux_conf() {
        return;
    }
    let srv = TestServer::unstarted();
    write(&home_conf(&srv), "set -g @a 1\nset -g @order a\n");
    write(&xdg_conf(&srv), "set -g @order b\n");
    start(&srv);
    assert_eq!(option(&srv, "@order"), "b");
    assert_eq!(option(&srv, "@a"), "1");
    kill(&srv);
}

#[test]
fn t10_with_xdg_config_home_set_the_same_file_loads_once() {
    if host_has_etc_tmux_conf() {
        return;
    }
    let srv = TestServer::unstarted();
    write(&xdg_conf(&srv), "set -ga @n x\n");
    let xdg = srv.home.join(".config");
    ok(&srv, Some(&xdg), &["new-session", "-d"]);
    assert_eq!(option(&srv, "@n"), "x");
    kill(&srv);
}

#[test]
fn t10_a_symlink_to_the_same_file_loads_twice_as_in_tmux() {
    if host_has_etc_tmux_conf() {
        return;
    }
    let srv = TestServer::unstarted();
    write(&xdg_conf(&srv), "set -ga @n x\n");
    symlink(xdg_conf(&srv), home_conf(&srv)).unwrap();
    start(&srv);
    assert_eq!(option(&srv, "@n"), "xx");
    kill(&srv);
}

#[test]
fn t10_a_grofe_config_toml_is_ignored_when_a_tmux_file_exists() {
    if host_has_etc_tmux_conf() {
        return;
    }
    let srv = TestServer::unstarted();
    write(
        &srv.home.join(".config/grofe/config.toml"),
        "status.left = \"TOML\"\n",
    );
    write(&home_conf(&srv), "set -g @a 1\n");
    start(&srv);
    assert_eq!(option(&srv, "@a"), "1");
    assert_ne!(option(&srv, "status-left"), "TOML");
    kill(&srv);
}

#[test]
fn t10_a_grofe_config_toml_is_used_when_no_tmux_file_exists() {
    if host_has_etc_tmux_conf() {
        return;
    }
    let srv = TestServer::unstarted();
    write(
        &srv.home.join(".config/grofe/config.toml"),
        "status.left = \"TOML\"\n",
    );
    start(&srv);
    assert_eq!(option(&srv, "status-left"), "TOML");
    kill(&srv);
}

#[test]
fn t11_the_grofe_identity_ignores_home_tmux_conf() {
    let srv = TestServer::unstarted();
    write(&home_conf(&srv), "set -g @a 1\nset -g status-left X\n");
    let mut cmd = srv.command();
    cmd.env_remove("GROFE_CONFIG")
        .env_remove("GROFE_CONFIG_DIR")
        .args(["new-session", "-d"]);
    let out = run_bounded(cmd, CMD_TIMEOUT);
    assert!(out.status.success(), "{out:?}");
    let mut cmd = srv.command();
    cmd.args(["show-options", "-gqv", "@a"]);
    assert_eq!(run_bounded(cmd, CMD_TIMEOUT).stdout.trim(), "");
}

#[test]
fn t11_config_dir_wins_over_home_tmux_conf_under_the_tmux_identity() {
    let srv = TestServer::unstarted();
    write(&home_conf(&srv), "set -g @a 1\n");
    let dir = srv.home.join("cust");
    write(&dir.join("config.toml"), "status.left = \"D\"\n");
    ok(
        &srv,
        None,
        &["--config-dir", dir.to_str().unwrap(), "new-session", "-d"],
    );
    assert_eq!(option(&srv, "status-left"), "D");
    assert_eq!(option(&srv, "@a"), "");
    kill(&srv);
}

#[test]
fn t11_grofe_config_env_wins_over_home_tmux_conf_under_the_tmux_identity() {
    let srv = TestServer::unstarted();
    write(&home_conf(&srv), "set -g @a 1\n");
    let file = srv.home.join("cust").join("c.toml");
    write(&file, "status.left = \"C\"\n");
    let out = tmux_env(
        &srv,
        None,
        &[("GROFE_CONFIG", file.as_path())],
        &["new-session", "-d"],
    );
    assert!(out.status.success(), "{}", out.stderr);
    assert_eq!(option(&srv, "status-left"), "C");
    assert_eq!(option(&srv, "@a"), "");
    kill(&srv);
}

#[test]
fn t11_f_wins_over_the_default_tmux_files() {
    let srv = TestServer::unstarted();
    write(&home_conf(&srv), "set -g @a 1\n");
    let file = srv.home.join("only.conf");
    write(&file, "set -g @b 2\n");
    ok(
        &srv,
        None,
        &["-f", file.to_str().unwrap(), "new-session", "-d"],
    );
    assert_eq!(option(&srv, "@b"), "2");
    assert_eq!(option(&srv, "@a"), "");
    kill(&srv);
}
