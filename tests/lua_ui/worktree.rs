//! Worktree grouping (AC16): sessions whose cwd is a git worktree sit under
//! their main repo, found by git common dir, with the branch as row label.

use std::path::PathBuf;

use crate::common::TestServer;
use crate::helpers::{attach, press, server};
use crate::support::{WAIT, git, write};

struct Repos {
    _root: tempfile::TempDir,
    main: PathBuf,
    wt_a: PathBuf,
    wt_b: PathBuf,
    plain: PathBuf,
    nogit: PathBuf,
}

fn repos() -> Repos {
    let root = tempfile::tempdir().expect("tempdir");
    let base = root.path().canonicalize().expect("canonicalize");
    let main = base.join("bifurcrepo");
    write(&main.join("f.txt"), "x");
    git(&main, &["init", "-q", "-b", "main"]);
    git(&main, &["add", "."]);
    git(&main, &["commit", "-q", "-m", "test: init"]);
    let wt_a = base.join("wt-a");
    let wt_b = base.join("wt-b");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat-a",
            wt_a.to_str().expect("utf8"),
        ],
    );
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat-b",
            wt_b.to_str().expect("utf8"),
        ],
    );
    let plain = base.join("plainrepo");
    write(&plain.join("f.txt"), "x");
    git(&plain, &["init", "-q", "-b", "trunk"]);
    git(&plain, &["add", "."]);
    git(&plain, &["commit", "-q", "-m", "test: init"]);
    let nogit = base.join("nogitdir");
    write(&nogit.join("f.txt"), "x");
    Repos {
        _root: root,
        main,
        wt_a,
        wt_b,
        plain,
        nogit,
    }
}

/// Sessions in creation order: aa-wt1, bb-plain, cc-wt2, dd-nogit, each
/// with one agent. Ungrouped, bb-plain would sit between the worktrees.
fn sessions(srv: &TestServer, r: &Repos) {
    for (name, dir) in [
        ("aa-wt1", &r.wt_a),
        ("bb-plain", &r.plain),
        ("cc-wt2", &r.wt_b),
        ("dd-nogit", &r.nogit),
    ] {
        srv.cmd(&[
            "new-session",
            "-d",
            "-s",
            name,
            "-x",
            "80",
            "-y",
            "24",
            "-c",
            dir.to_str().expect("utf8"),
            "cat",
        ])
        .expect("new-session");
        let pane = srv
            .cmd(&["display-message", "-p", "-t", name, "#{pane_id}"])
            .expect("pane")
            .trim()
            .to_string();
        srv.cmd(&["agent", "report", "idle", "-t", &pane, "--kind", "claude"])
            .expect("agent report");
    }
}

fn line_of(text: &str, needle: &str) -> usize {
    text.lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not on screen:\n{text}"))
}

/// The repo name sits above both branch rows, and the plain repo's session
/// is not between them.
fn assert_grouped(text: &str) {
    let (repo, a, b) = (
        line_of(text, "bifurcrepo"),
        line_of(text, "feat-a"),
        line_of(text, "feat-b"),
    );
    assert!(
        repo < a && repo < b,
        "repo header above its branches:\n{text}"
    );
    let (lo, hi) = (a.min(b), a.max(b));
    let plain = line_of(text, "bb-plain");
    assert!(
        !(lo < plain && plain < hi),
        "plain session between the worktrees:\n{text}"
    );
}

#[test]
fn two_worktrees_group_under_their_repo_in_the_sidebar() {
    let r = repos();
    let srv = server();
    sessions(&srv, &r);
    let mut a = attach(&srv, 160, 40);
    press(&mut a, "b");
    a.wait_until("rows", WAIT, |t| {
        t.contains("feat-a") && t.contains("feat-b")
    });
    assert_grouped(&a.text());
}

#[test]
fn plain_repo_and_non_git_dir_keep_their_own_rows() {
    let r = repos();
    let srv = server();
    sessions(&srv, &r);
    let mut a = attach(&srv, 160, 40);
    press(&mut a, "b");
    a.wait_until("rows", WAIT, |t| {
        t.contains("bb-plain") && t.contains("dd-nogit") && t.contains("feat-a")
    });
    let text = a.text();
    assert!(
        !text.contains("plainrepo\n"),
        "a plain repo is no group header"
    );
    assert!(text.contains("dd-nogit"), "{text}");
}

#[test]
fn a_session_in_the_main_checkout_joins_its_worktrees() {
    let r = repos();
    let srv = server();
    sessions(&srv, &r);
    srv.cmd(&[
        "new-session",
        "-d",
        "-s",
        "ee-main",
        "-x",
        "80",
        "-y",
        "24",
        "-c",
        r.main.to_str().expect("utf8"),
        "cat",
    ])
    .expect("new-session");
    agent_pane_in(&srv, "ee-main");
    let mut a = attach(&srv, 160, 40);
    press(&mut a, "b");
    a.wait_until("rows", WAIT, |t| {
        t.contains("feat-a") && t.contains("feat-b")
    });
    let text = a.text();
    let (repo, main_row) = (line_of(&text, "bifurcrepo"), line_of(&text, "ee-main"));
    let (a_row, b_row) = (line_of(&text, "feat-a"), line_of(&text, "feat-b"));
    let hi = a_row.max(b_row).max(main_row);
    assert!(
        repo < a_row.min(b_row).min(main_row) && hi - repo < 12,
        "{text}"
    );
}

fn agent_pane_in(srv: &TestServer, session: &str) {
    let pane = srv
        .cmd(&["display-message", "-p", "-t", session, "#{pane_id}"])
        .expect("pane")
        .trim()
        .to_string();
    srv.cmd(&["agent", "report", "idle", "-t", &pane, "--kind", "claude"])
        .expect("agent report");
}

#[test]
fn the_picker_groups_worktrees_too() {
    let r = repos();
    let srv = server();
    sessions(&srv, &r);
    let mut a = attach(&srv, 160, 40);
    press(&mut a, "U");
    a.wait_until("rows", WAIT, |t| {
        t.contains("feat-a") && t.contains("feat-b")
    });
    assert_grouped(&a.text());
}

/// A grouped row is its branch, then the `agent-sidebar-format` label, so a
/// pane option such as `@issue` shows in the sidebar and the picker.
fn tag_and_format(srv: &TestServer) {
    for (name, issue) in [("aa-wt1", "ENG-1"), ("cc-wt2", "ENG-2")] {
        let pane = srv
            .cmd(&["display-message", "-p", "-t", name, "#{pane_id}"])
            .expect("pane")
            .trim()
            .to_string();
        srv.cmd(&["set-option", "-p", "-t", &pane, "@issue", issue])
            .expect("tag pane");
    }
    srv.cmd(&["set-option", "-g", "agent-sidebar-format", "#{@issue}"])
        .expect("set format");
}

#[test]
fn the_sidebar_row_is_the_branch_then_the_formatted_label() {
    let r = repos();
    let srv = server();
    sessions(&srv, &r);
    tag_and_format(&srv);
    let mut a = attach(&srv, 160, 40);
    press(&mut a, "b");
    a.wait_until("rows", WAIT, |t| {
        t.contains("feat-a · ENG-1") && t.contains("feat-b · ENG-2")
    });
}

#[test]
fn the_picker_row_is_the_branch_then_the_formatted_label() {
    let r = repos();
    let srv = server();
    sessions(&srv, &r);
    tag_and_format(&srv);
    let mut a = attach(&srv, 160, 40);
    press(&mut a, "U");
    a.wait_until("rows", WAIT, |t| {
        t.contains("feat-a · ENG-1") && t.contains("feat-b · ENG-2")
    });
}
