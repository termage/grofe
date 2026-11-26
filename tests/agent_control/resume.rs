//! AC15: after a restore, an agent pane with a resume command and a
//! session id runs that command. Exists today; these tests prove it with a
//! fake kind whose resume template is the vendored `fake-agent` fixture.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::common::TestServer;
use crate::helpers::{WAIT, write_config};

fn transcript(home: &Path, id: &str) -> String {
    fs::read_to_string(home.join(format!("{id}.transcript"))).unwrap_or_default()
}

/// A restoring server, `path_env` pinned as in restore_resume.rs, whose
/// kind `fake` runs and resumes the `fake-agent` fixture.
fn start(dir: &Path) -> (TestServer, std::path::PathBuf) {
    let agent = dir.join("fake-agent");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-agent"),
        &agent,
    )
    .expect("copy fake-agent");
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o755)).expect("chmod");
    let home = dir.join("agent-home");
    let srv = TestServer::builder()
        .restore_on_start(true)
        .path_env("/usr/bin:/bin")
        .start();
    write_config(
        &srv,
        &agent,
        &format!("resume = \"{} --resume {{id}}\"", agent.display()),
    );
    let conf = srv.config_home.join("grofe").join("config.toml");
    fs::write(
        conf,
        format!("[environment]\nFAKE_AGENT_HOME = \"{}\"\n", home.display()),
    )
    .expect("write config.toml");
    // The config is read at start: restart once so it applies.
    let srv = srv.restart();
    srv.cmd(&[
        "new-session",
        "-d",
        "-s",
        "main",
        "-x",
        "80",
        "-y",
        "24",
        "cat",
    ])
    .expect("new-session");
    (srv, home)
}

#[test]
fn a_reported_session_id_is_resumed_after_a_crash() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (srv, home) = start(dir.path());
    let agent = dir.path().join("fake-agent");
    let pane = srv
        .cmd(&[
            "agent",
            "new",
            "fake",
            "-P",
            "-d",
            "--",
            agent.to_str().expect("utf8"),
            "--session-id",
            "s1",
        ])
        .unwrap_or_else(|e| panic!("agent new: {e}; log:\n{}", srv.log()))
        .trim()
        .to_string();
    srv.wait_for(|| transcript(&home, "s1") == "launch s1\n", WAIT)
        .unwrap_or_else(|e| panic!("{e}: launched; log:\n{}", srv.log()));
    srv.cmd(&[
        "agent",
        "report",
        "idle",
        "-t",
        &pane,
        "--kind",
        "fake",
        "--session-id",
        "s1",
    ])
    .expect("agent report");
    let want = format!("{} --resume s1", agent.display());
    srv.wait_for(
        || {
            srv.cmd(&["display-message", "-p", "-t", &pane, "#{pane_resume}"])
                .is_ok_and(|r| r.trim() == want)
        },
        WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: the report sets the pane's resume"));
    // Wait for the snapshot to carry it before the crash.
    srv.wait_for(
        || fs::read_to_string(srv.snapshot_path()).is_ok_and(|t| t.contains("--resume")),
        WAIT,
    )
    .unwrap_or_else(|e| panic!("{e}: the resume is saved"));
    srv.kill_hard();
    let srv = srv.restart();
    srv.wait_for(|| transcript(&home, "s1") == "launch s1\nresume s1\n", WAIT)
        .unwrap_or_else(|e| {
            panic!(
                "{e}: transcript {:?}; log:\n{}",
                transcript(&home, "s1"),
                srv.log()
            )
        });
}
