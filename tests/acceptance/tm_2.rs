//! TM.2 acceptance: selection and copying in copy mode, replayed from
//! tmux 3.7b's goldens (`tests/fixtures/compat/golden/tm_2/`). Each
//! golden checks `show-buffer` and the buffer's exact size
//! (`list-buffers -F '#{buffer_size}'`), so the copied bytes match tmux's
//! byte for byte, trailing newline included.
//!
//! Each pane signals `wait-for ready` once its output is written, through
//! tmux there and through grofe here (`{grofe}`, the binary under test,
//! which finds the server through the pane's `$GROFE`).

use crate::common;

use std::fs;

use common::{CMD_TIMEOUT, TestServer, try_wait_until};

use crate::golden::{Step, fixtures, load};
use crate::harness::Harness;

fn golden_text(name: &str) -> String {
    fs::read_to_string(fixtures().join(format!("golden/tm_2/{name}.golden"))).unwrap()
}

/// Runs golden `name` on a fresh server, comparing each command with tmux.
fn replay(name: &str) {
    let srv = TestServer::start();
    let mut client: Option<Harness> = None;
    // Typed keys reach the server in order but after the CLI's commands
    // may: the first command after them is retried until it shows what
    // tmux's did, instead of sleeping.
    let mut typed = false;
    for step in load(name, &golden_text(name)) {
        match step {
            Step::Run { line, args, want } => {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                let run = || {
                    let out = srv.run(&args);
                    let mut stdout = out.stdout;
                    if !stdout.is_empty() && !stdout.ends_with('\n') {
                        stdout.push('\n');
                    }
                    (stdout, out.stderr, out.status.code().unwrap_or(-1))
                };
                if std::mem::take(&mut typed) {
                    let _ = try_wait_until(|| run() == want, CMD_TIMEOUT);
                }
                assert_eq!(run(), want, "{name}: {line}");
            }
            Step::Client(cols, rows) => {
                let a = Harness::spawn_attach_with_env(srv.command(), "main", &[], cols, rows);
                try_wait_until(
                    || !srv.cmd(&["list-clients"]).unwrap_or_default().is_empty(),
                    CMD_TIMEOUT,
                )
                .expect("client attached");
                client = Some(a);
            }
            Step::Keys(bytes) => {
                let a = client.as_mut().expect("keys before a client");
                a.send(&bytes);
                let _ = a.text();
                typed = true;
            }
        }
    }
}

/// Cases 1 and 6: a line wrapped three times at 80 columns copies as one
/// line; trailing blanks (default or coloured) go, colours copy as text,
/// tabs come back as tabs, a line selection keeps its newline.
#[test]
fn wrapped_lines_copy_as_the_golden() {
    replay("wrap");
}

/// Case 2: the same line through a resize to 50 columns. tmux clears the
/// selection on a resize; the line selected again copies the same text.
#[test]
fn a_reflowing_resize_copies_as_the_golden() {
    replay("resize");
}

/// Case 3: rectangles over CJK, emoji, a combining mark and ASCII.
#[test]
fn rectangles_copy_as_the_golden() {
    replay("rect");
}

/// Case 4: `select-word` on `foo/bar-baz.rs` with the default
/// `word-separators`, across a wrap, and with none.
#[test]
fn select_word_copies_as_the_golden() {
    replay("word");
}

/// Case 5: `append-selection` onto a prior copy, and the other copy
/// commands (`-no-clear`, `-and-cancel`, `-P`, `copy-end-of-line`,
/// `other-end`, `clear-selection`).
#[test]
fn append_and_copy_commands_equal_the_golden() {
    replay("append");
}

/// `copy-line` and its `-and-cancel` form, `rectangle-on`,
/// `rectangle-off` and `stop-selection`.
#[test]
fn copy_line_rectangle_and_stop_equal_the_golden() {
    replay("commands");
}

/// The selection rows typed on a pty-attached client.
#[test]
fn selection_keys_through_a_client_equal_the_golden() {
    replay("keys");
}

/// Copying a whole 100 000-line scrollback works: 100 000 lines of 79
/// characters, then the empty cursor row, which a vi selection ending
/// past its line's text keeps as a final newline.
#[test]
fn a_whole_100k_line_scrollback_copies() {
    let srv = TestServer::start();
    srv.cmd(&["set", "-g", "history-limit", "200000"]).unwrap();
    srv.cmd(&[
        "new-session",
        "-d",
        "-s",
        "main",
        "-x",
        "80",
        "-y",
        "24",
        "yes 0000000000000000000000000000000000000000000000000000000000000000000000000000000 \
         | head -n 100000; exec sleep 100000",
    ])
    .unwrap();
    let rows = || {
        srv.cmd(&["display", "-p", "-t", "main", "#{history_size}"])
            .unwrap_or_default()
    };
    try_wait_until(
        || rows().trim() == "99977",
        std::time::Duration::from_secs(120),
    )
    .unwrap_or_else(|_| panic!("history_size {}", rows()));
    for cmd in [
        &["copy-mode", "-t", "main"][..],
        &["send-keys", "-t", "main", "-X", "history-top"],
        &["send-keys", "-t", "main", "-X", "begin-selection"],
        &["send-keys", "-t", "main", "-X", "history-bottom"],
        &["send-keys", "-t", "main", "-X", "copy-selection"],
    ] {
        // This checks what is copied, not how fast: a debug build on a
        // loaded host can take longer than CMD_TIMEOUT over 100k lines.
        let mut c = srv.command();
        c.args(cmd);
        let out = common::run_bounded(c, std::time::Duration::from_secs(120));
        assert!(out.status.success(), "{cmd:?}: {}", out.stderr);
    }
    assert_eq!(
        srv.cmd(&["list-buffers", "-F", "#{buffer_name} #{buffer_size}"]),
        Ok(format!("buffer0 {}\n", 100_000 * 80 + 1))
    );
}
