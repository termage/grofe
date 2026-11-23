//! tmux 3.7b's special pane tokens (`{left-of}`, `{top}`, ...), against
//! grofe run as `tmux` on a private socket.

use crate::fe_1::grofe_cli;

/// canopy's "Swap left" is `swap-pane -d -t "{left-of}"`: the token names
/// the active pane's left neighbour, and `-d` leaves the active pane alone.
#[test]
fn swap_pane_swaps_with_the_pane_a_token_names() {
    let dir = tempfile::tempdir().unwrap();
    let cli = grofe_cli(dir.path());
    let run = |words: &[&str]| {
        let args: Vec<String> = words.iter().map(|s| s.to_string()).collect();
        let (status, out, err) = cli.run(&args);
        (status.success(), out, err)
    };
    let panes = || run(&["list-panes", "-F", "#{pane_id}#{?pane_active,*,}"]).1;
    for words in [
        &["new-session", "-d", "-x", "80", "-y", "24", "-s", "main"][..],
        &["split-window", "-h", "-d", "-t", "main"],
    ] {
        let (ok, _, err) = run(words);
        assert!(ok, "{err}");
    }
    assert_eq!(panes(), "%0*\n%1\n");

    // %1 is not active: `{right-of}` of the active %0 is %1.
    let (ok, _, err) = run(&["swap-pane", "-d", "-t", "{right-of}"]);
    assert!(ok, "{err}");
    assert_eq!(panes(), "%1\n%0*\n");

    // The active pane is now on the right: `{left-of}` swaps it back.
    let (ok, _, err) = run(&["swap-pane", "-d", "-t", "{left-of}"]);
    assert!(ok, "{err}");
    assert_eq!(panes(), "%0*\n%1\n");

    let (_, out, _) = run(&["display", "-p", "-t", "{right}", "#{pane_id}"]);
    assert_eq!(out, "%1\n");
    // The plain words, in any case, bare or after `window.`.
    for (target, pane) in [
        ("right", "%1"),
        ("Right", "%1"),
        ("BOTTOM-RIGHT", "%1"),
        ("left", "%0"),
        ("top-left", "%0"),
        ("main:.Right", "%1"),
        ("main:.Left", "%0"),
    ] {
        let (_, out, err) = run(&["display", "-p", "-t", target, "#{pane_id}"]);
        assert_eq!(out, format!("{pane}\n"), "-t {target}: {err}");
    }
    let (ok, _, err) = run(&["select-pane", "-t", "up-of"]);
    assert!(!ok);
    assert_eq!(err, "can't find pane: up-of\n");
    let (ok, _, err) = run(&["select-pane", "-t", "{up-of}"]);
    assert!(!ok);
    assert_eq!(err, "can't find pane: {up-of}\n");
    let (ok, _, err) = run(&["select-pane", "-t", "{mouse}"]);
    assert!(!ok);
    assert_eq!(err, "no mouse target\n");
    let _ = run(&["kill-server"]);
}
