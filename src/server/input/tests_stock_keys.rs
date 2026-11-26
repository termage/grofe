//! tmux 3.7b's stock marking, clock, menu and float keys under grofe
//! identity, and renaming a pane.

use super::tests::{k, words};
use super::*;

fn stock(key: &str) -> (&'static str, &'static str) {
    TMUX_STOCK
        .iter()
        .find(|(name, ..)| *name == key)
        .map(|(_, _, note, text)| (*note, *text))
        .unwrap_or_else(|| panic!("no stock {key}"))
}

/// grofe identity binds tmux 3.7b's `m`, `M`, `t` and `*` as tmux does,
/// with its notes.
#[test]
fn grofe_identity_binds_tmux_s_marking_clock_and_float_keys() {
    let t = default_bindings();
    for key in ["m", "M", "t", "*"] {
        let (note, text) = stock(key);
        let b = t.exact(PREFIX_TABLE, &k(key)).expect(key);
        assert_eq!(words(b), text.split(' ').collect::<Vec<_>>(), "{key}");
        assert_eq!(b.desc.as_deref(), Some(note), "{key}");
        assert_eq!(b.dialect, Dialect::Grofe, "{key}");
        assert_eq!(b.group.as_deref(), Some("pane"), "{key}");
    }
}

/// `prefix <` is tmux's window menu, on the window's status line entry.
#[test]
fn prefix_less_than_opens_the_window_menu() {
    let t = default_bindings();
    let (note, text) = stock("<");
    let b = t.exact(PREFIX_TABLE, &k("<")).expect("<");
    assert_eq!(b.desc.as_deref(), Some(note));
    assert_eq!(b.dialect, Dialect::Tmux);
    assert_eq!(b.cmds, crate::cmd::split_command_string(text).unwrap());
    assert_eq!(b.group.as_deref(), Some("window"));
    let w = words(b);
    assert_eq!(&w[..7], ["display-menu", "-T", w[2], "-x", "W", "-y", "W"]);
}

/// `prefix >` is grofe identity's pane menu, the right-click one, on the
/// active pane: below it, its items acting on it (no mouse `=` target).
#[test]
fn prefix_greater_than_opens_grofe_s_pane_menu_on_the_active_pane() {
    let t = default_bindings();
    let b = t.exact(PREFIX_TABLE, &k(">")).expect(">");
    assert_eq!(b.desc.as_deref(), Some(stock(">").0));
    assert_eq!(b.dialect, Dialect::Grofe);
    assert_eq!(b.group.as_deref(), Some("pane"));
    let w = words(b);
    assert_eq!(&w[..7], ["display-menu", "-T", w[2], "-x", "P", "-y", "P"]);
    for label in ["Float", "Unfloat", "Rename", "Kill"] {
        assert!(w.iter().any(|a| a.contains(label)), "{label}: {w:?}");
    }
    assert!(
        w.iter().all(|a| !a.contains("'-t' '='") && *a != "="),
        "{w:?}"
    );
}

/// grofe identity's pane menu, on a right click and on `prefix >`,
/// renames the pane: `n` prompts with its title and sets it.
#[test]
fn grofe_s_pane_menu_renames_the_pane() {
    let rename = r###"Rename n { select-pane -t = ; command-prompt -I "##{pane_title}" { select-pane -T "%%" } }"###;
    assert!(grofe_pane_menu().contains(rename), "{}", grofe_pane_menu());
    let t = default_bindings();
    let menu = t.exact(ROOT_TABLE, &k("MouseDown3Pane")).expect("menu");
    let parsed = crate::cmd::split_command_string(&grofe_pane_menu()).unwrap();
    assert_eq!(menu.cmds, parsed);
}

/// grofe's own `prefix e` renames the active pane through a prompt; tmux
/// 3.7b binds no `e`, so tmux identity has none.
#[test]
fn prefix_e_renames_the_active_pane_under_grofe_identity_only() {
    let mut t = default_bindings();
    let e = t.exact(PREFIX_TABLE, &k("e")).expect("grofe's e");
    assert_eq!(
        words(e),
        [
            "command-prompt",
            "-I",
            "#{pane_title}",
            "select-pane -T \"%%\""
        ]
    );
    assert_eq!(e.desc.as_deref(), Some("Rename the active pane"));
    assert_eq!(e.group.as_deref(), Some("pane"));
    t.use_tmux_stock();
    assert!(t.exact(PREFIX_TABLE, &k("e")).is_none());
}

/// Under tmux identity the new keys are tmux's own rows, byte for byte,
/// and the right-click menus are tmux's stock ones.
#[test]
fn tmux_identity_keeps_tmux_s_stock_rows_for_the_new_keys() {
    let mut t = default_bindings();
    t.use_tmux_stock();
    for key in ["m", "M", "t", "*", "<", ">"] {
        let (note, text) = stock(key);
        let b = t.exact(PREFIX_TABLE, &k(key)).expect(key);
        assert_eq!(b.shown.as_deref(), Some(text), "{key}");
        assert_eq!(b.desc.as_deref(), Some(note), "{key}");
        assert_eq!(b.dialect, Dialect::Tmux, "{key}");
    }
    for name in ["MouseDown3Pane", "MouseDown3Status"] {
        let text = TMUX_STOCK_ROOT.iter().find(|(n, _)| *n == name).unwrap().1;
        let b = t.exact(ROOT_TABLE, &k(name)).expect(name);
        assert_eq!(b.shown.as_deref(), Some(text), "{name}");
        assert!(!text.contains("pane_title"), "{name}");
    }
}
