//! The tmux config files a server started as `tmux` reads when it was given
//! no config at all (spec 2026-11-25-config-dir-and-shrink, A11 and A12).
//! When one exists the server behaves as if the CLI had passed each existing
//! one with `-f`; when none does, grofe's TOML chain loads as before.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// tmux's system config file.
const ETC_TMUX_CONF: &str = "/etc/tmux.conf";

/// tmux's default config paths, in tmux 3.7b's order: `etc`, `~/.tmux.conf`,
/// `$XDG_CONFIG_HOME/tmux/tmux.conf` (only when that is set and not empty),
/// `~/.config/tmux/tmux.conf`. A path whose string equals an earlier one is
/// dropped; another spelling of the same path (a trailing slash) or a
/// symlink to the same file is not, as in tmux.
pub fn candidates(etc: &Path, home: Option<&Path>, xdg_config_home: Option<&str>) -> Vec<PathBuf> {
    let mut all = vec![etc.to_path_buf()];
    if let Some(home) = home {
        all.push(home.join(".tmux.conf"));
    }
    if let Some(xdg) = xdg_config_home.filter(|x| !x.is_empty()) {
        all.push(PathBuf::from(format!("{xdg}/tmux/tmux.conf")));
    }
    if let Some(home) = home {
        all.push(home.join(".config").join("tmux").join("tmux.conf"));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for path in all {
        if !out.iter().any(|p| p.as_os_str() == path.as_os_str()) {
            out.push(path);
        }
    }
    out
}

/// `~`: `home_var` (`$HOME`) when set and not empty, else `passwd` (the
/// passwd home).
fn home(home_var: Option<OsString>, passwd: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    home_var
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(passwd)
}

fn passwd_home() -> Option<PathBuf> {
    nix::unistd::User::from_uid(nix::unistd::getuid())
        .ok()
        .flatten()
        .map(|u| u.dir)
}

/// The [`candidates`] that exist, or `None` when none does.
fn existing(
    etc: &Path,
    home: Option<&Path>,
    xdg_config_home: Option<&str>,
) -> Option<Vec<PathBuf>> {
    let found: Vec<PathBuf> = candidates(etc, home, xdg_config_home)
        .into_iter()
        .filter(|p| p.exists())
        .collect();
    (!found.is_empty()).then_some(found)
}

/// The existing default tmux files of a server started as `tmux` without
/// `-f` or a grofe config (`--config`, `--config-dir`, or their variables,
/// which the starting CLI resolved into those), or `None` when there is no
/// such file. Decided once, so a reload reads the same files.
pub fn files() -> Option<Vec<PathBuf>> {
    static FILES: OnceLock<Option<Vec<PathBuf>>> = OnceLock::new();
    FILES
        .get_or_init(|| {
            let explicit = crate::config::process_config().is_some()
                || crate::config::process_config_dir().is_some();
            if !super::started_as_tmux() || explicit {
                return None;
            }
            let home = home(std::env::var_os("HOME"), passwd_home);
            let xdg = std::env::var("XDG_CONFIG_HOME").ok();
            existing(Path::new(ETC_TMUX_CONF), home.as_deref(), xdg.as_deref())
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    const ETC: &str = "/etc/tmux.conf";

    #[test]
    fn the_candidates_are_tmux_order_without_xdg() {
        let got = candidates(Path::new(ETC), Some(Path::new("/h")), None);
        let want = paths(&[ETC, "/h/.tmux.conf", "/h/.config/tmux/tmux.conf"]);
        assert_eq!(got, want);
    }

    #[test]
    fn an_empty_xdg_config_home_is_unset() {
        let h = Some(Path::new("/h"));
        assert_eq!(
            candidates(Path::new(ETC), h, Some("")),
            candidates(Path::new(ETC), h, None)
        );
    }

    #[test]
    fn xdg_comes_before_the_home_config_and_a_repeat_is_dropped() {
        let h = Some(Path::new("/h"));
        let got = candidates(Path::new(ETC), h, Some("/x"));
        assert_eq!(got[2], PathBuf::from("/x/tmux/tmux.conf"));
        assert_eq!(got.len(), 4);
        assert_eq!(candidates(Path::new(ETC), h, Some("/h/.config")).len(), 3);
    }

    #[test]
    fn a_trailing_slash_is_another_string_so_both_stay() {
        let got = candidates(Path::new(ETC), Some(Path::new("/h")), Some("/h/.config/"));
        assert_eq!(
            got,
            paths(&[
                ETC,
                "/h/.tmux.conf",
                "/h/.config//tmux/tmux.conf",
                "/h/.config/tmux/tmux.conf",
            ])
        );
    }

    #[test]
    fn home_unset_or_empty_falls_back_to_the_passwd_home() {
        let pw = || Some(PathBuf::from("/pw"));
        assert_eq!(home(None, pw), Some(PathBuf::from("/pw")));
        assert_eq!(home(Some(OsString::new()), pw), Some(PathBuf::from("/pw")));
        assert_eq!(home(Some("/h".into()), pw), Some(PathBuf::from("/h")));
    }

    #[test]
    fn only_existing_files_count_and_none_is_none() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let etc = root.path().join("etc-tmux.conf");
        assert_eq!(existing(&etc, Some(&home), None), None);
        std::fs::write(&etc, "").unwrap();
        assert_eq!(existing(&etc, Some(&home), None), Some(vec![etc.clone()]));
        let user = home.join(".tmux.conf");
        std::fs::write(&user, "").unwrap();
        assert_eq!(existing(&etc, Some(&home), None), Some(vec![etc, user]));
    }
}
