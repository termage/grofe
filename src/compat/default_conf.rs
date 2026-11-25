//! The tmux config files a server started as `tmux` reads when it was given
//! no config at all (spec 2026-11-25-config-dir-and-shrink, A11 and A12).
//! When one exists the server behaves as if the CLI had passed each existing
//! one with `-f`; when none does, grofe's TOML chain loads as before.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// tmux's default config paths, in tmux 3.7b's order: `/etc/tmux.conf`,
/// `~/.tmux.conf`, `$XDG_CONFIG_HOME/tmux/tmux.conf` (only when that is set
/// and not empty), `~/.config/tmux/tmux.conf`. A path whose string equals an
/// earlier one is dropped; a symlink to the same file is not.
pub fn candidates(home: Option<&Path>, xdg_config_home: Option<&str>) -> Vec<PathBuf> {
    let mut all = vec![PathBuf::from("/etc/tmux.conf")];
    if let Some(home) = home {
        all.push(home.join(".tmux.conf"));
    }
    if let Some(xdg) = xdg_config_home.filter(|x| !x.is_empty()) {
        all.push(Path::new(xdg).join("tmux").join("tmux.conf"));
    }
    if let Some(home) = home {
        all.push(home.join(".config").join("tmux").join("tmux.conf"));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for path in all {
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// `~`: `$HOME` when set and not empty, else the passwd home.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            nix::unistd::User::from_uid(nix::unistd::getuid())
                .ok()
                .flatten()
                .map(|u| u.dir)
        })
}

/// Whether this process was given a grofe config: `--config` (passed to
/// the server as its argument), `--config-dir`, `GROFE_CONFIG` or
/// `GROFE_CONFIG_DIR`, a variable counting only when not empty.
fn explicit_config() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    crate::config::process_config().is_some()
        || crate::config::process_config_dir().is_some()
        || set("GROFE_CONFIG")
        || set("GROFE_CONFIG_DIR")
}

/// The existing default tmux files of a server started as `tmux` without
/// `-f` or a grofe config, or `None` when there is no such file. Decided
/// once, so a reload reads the same files.
pub fn files() -> Option<Vec<PathBuf>> {
    static FILES: OnceLock<Option<Vec<PathBuf>>> = OnceLock::new();
    FILES
        .get_or_init(|| {
            if !super::started_as_tmux() || explicit_config() {
                return None;
            }
            let xdg = std::env::var("XDG_CONFIG_HOME").ok();
            let found: Vec<PathBuf> = candidates(home().as_deref(), xdg.as_deref())
                .into_iter()
                .filter(|p| p.exists())
                .collect();
            (!found.is_empty()).then_some(found)
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_candidates_are_tmux_order_without_xdg() {
        let got = candidates(Some(Path::new("/h")), None);
        let want: Vec<PathBuf> = [
            "/etc/tmux.conf",
            "/h/.tmux.conf",
            "/h/.config/tmux/tmux.conf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn an_empty_xdg_config_home_is_unset() {
        assert_eq!(
            candidates(Some(Path::new("/h")), Some("")),
            candidates(Some(Path::new("/h")), None)
        );
    }

    #[test]
    fn xdg_comes_before_the_home_config_and_a_repeat_is_dropped() {
        let got = candidates(Some(Path::new("/h")), Some("/x"));
        assert_eq!(got[2], PathBuf::from("/x/tmux/tmux.conf"));
        assert_eq!(got.len(), 4);
        let same = candidates(Some(Path::new("/h")), Some("/h/.config"));
        assert_eq!(same.len(), 3);
    }
}
