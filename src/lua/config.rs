//! `grofe.config(name, dir?)`: a pack's own settings file, from the user
//! and from a project.

use mlua::{Lua, Table};
use std::path::{Path, PathBuf};

use super::json::{from_json, merge};
use super::require;

fn err(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(e.to_string())
}

pub fn install(lua: &Lua, g: &Table) -> mlua::Result<()> {
    g.set(
        "config",
        lua.create_function(|lua, (name, dir): (String, Option<String>)| {
            if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
                return Err(err(format!("grofe.config: {name:?} is not a usable name")));
            }
            let file = format!("{name}.toml");
            let mut merged = serde_json::json!({});
            if let Some(dir) = require::config_dir(lua)? {
                merge(&mut merged, read(&dir.join(&file))?);
            }
            if let Some(root) = dir.as_deref().and_then(|d| git_root(Path::new(d))) {
                merge(&mut merged, read(&root.join(".grofe").join(&file))?);
            }
            from_json(lua, &merged)
        })?,
    )
}

/// The nearest folder at or above `dir` with a `.git`.
fn git_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

/// `path` as JSON; `{}` when it does not exist.
fn read(path: &Path) -> mlua::Result<serde_json::Value> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(serde_json::json!({})),
        Err(e) => return Err(err(format!("{}: {e}", path.display()))),
    };
    let table: toml::Table =
        toml::from_str(&text).map_err(|e| err(format!("{}: {e}", path.display())))?;
    serde_json::to_value(table).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_root_finds_the_nearest_repo_and_none_outside() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("r/.git")).unwrap();
        std::fs::create_dir_all(d.path().join("r/a/b")).unwrap();
        assert_eq!(git_root(&d.path().join("r/a/b")), Some(d.path().join("r")));
        assert_eq!(git_root(&d.path().join("nothing")), None);
    }

    #[test]
    fn test_read_names_the_file_and_line() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("t.toml");
        std::fs::write(&f, "a = 1\nb = = 2\n").unwrap();
        let e = read(&f).unwrap_err().to_string();
        assert!(e.contains("t.toml") && e.contains("line 2"), "{e}");
    }
}
