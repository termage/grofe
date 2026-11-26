//! A2a bullet 2: `agent new <kind> -c <dir> [-e K=V] -- <args...>` with `-P`.

use crate::common::wait_for_file;
use crate::helpers::server;

#[test]
fn agent_new_prints_the_pane_id_and_runs_in_dir_with_env() {
    let srv = server();
    let dir = srv.home.join("work");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let out = dir.join("out");
    let script = format!("echo \"$GL_K:$(pwd)\" > {}; sleep 30", out.display());
    let id = srv
        .cmd(&[
            "agent",
            "new",
            "claude",
            "-P",
            "-c",
            dir.to_str().expect("utf8"),
            "-e",
            "GL_K=v",
            "--",
            "sh",
            "-c",
            &script,
        ])
        .unwrap_or_else(|e| panic!("agent new: {e}; log:\n{}", srv.log()));
    let id = id.trim();
    assert!(
        id.starts_with('%') && id[1..].chars().all(|c| c.is_ascii_digit()),
        "{id:?}"
    );
    let panes = srv
        .cmd(&["list-panes", "-a", "-F", "#{pane_id}"])
        .expect("list-panes");
    assert!(panes.lines().any(|l| l == id), "{id} not in {panes:?}");
    let text = wait_for_file(&out);
    let real = std::fs::canonicalize(&dir).expect("canon");
    assert_eq!(text.trim(), format!("v:{}", real.display()));
}

#[test]
fn agent_new_env_without_equals_is_an_error() {
    let srv = server();
    let before = srv.cmd(&["list-panes", "-a"]).expect("list-panes");
    let err = srv
        .cmd(&[
            "agent", "new", "claude", "-P", "-c", "/", "-e", "NOEQUALS", "--", "sleep", "30",
        ])
        .expect_err("-e without = must fail");
    assert!(err.contains("K=V") || err.contains("="), "{err}");
    assert_eq!(srv.cmd(&["list-panes", "-a"]).expect("list-panes"), before);
}
