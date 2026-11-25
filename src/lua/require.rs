//! `require` for the user's config folder: a Rust function that replaces
//! mlua's global `require`, so `init.luau` can split itself into modules.

use mlua::{Lua, Table, Value};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The canonical config folder, set by the init load.
const DIR: &str = "grofe_require_dir";
/// Module name to the value it returned, for this load.
const CACHE: &str = "grofe_require_cache";
/// Modules whose chunk is running, to catch cycles.
const LOADING: &str = "grofe_require_loading";
/// mlua's own `require`, used while no folder is set (plugin and pack code).
const FALLBACK: &str = "grofe_require_fallback";

fn err(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(e.to_string())
}

/// Replaces the global `require`. Call before `sandbox(true)`.
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let original: Value = lua.globals().get("require")?;
    lua.set_named_registry_value(FALLBACK, original)?;
    let f = lua.create_function(|lua, name: Value| {
        let dir = match lua.named_registry_value::<Value>(DIR)? {
            Value::String(s) => PathBuf::from(OsStr::from_bytes(&s.as_bytes())),
            _ => {
                return match lua.named_registry_value::<Value>(FALLBACK)? {
                    Value::Function(f) => f.call::<Value>(name),
                    _ => Err(err("require: no config folder")),
                };
            }
        };
        let Value::String(name) = name else {
            return Err(err("require takes a module name"));
        };
        require(lua, &dir, &name.to_str()?)
    })?;
    lua.globals().set("require", f)
}

/// Points `require` at `dir` and starts an empty module cache.
pub fn set_dir(lua: &Lua, dir: &Path) -> mlua::Result<()> {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let bytes = lua.create_string(dir.as_os_str().as_bytes())?;
    lua.set_named_registry_value(DIR, bytes)?;
    lua.set_named_registry_value(CACHE, lua.create_table()?)?;
    lua.set_named_registry_value(LOADING, lua.create_table()?)
}

/// `name` as a path relative to the folder, or why it is refused.
fn relative(name: &str) -> Result<String, String> {
    let bare = name.strip_prefix("./").unwrap_or(name);
    if bare.is_empty() {
        return Err("require: empty module name".into());
    }
    if bare.contains("..") {
        return Err(format!("require: '{name}' may not contain '..'"));
    }
    if bare.starts_with('/') {
        return Err(format!("require: '{name}' may not be an absolute path"));
    }
    if bare.ends_with(".luau") || bare.ends_with(".lua") {
        return Err(format!(
            "require: '{name}' has a file extension; leave it off"
        ));
    }
    let rel = bare.replace('.', "/");
    if rel.split('/').any(str::is_empty) {
        return Err(format!("require: '{name}' has an empty path segment"));
    }
    Ok(rel)
}

/// The paths tried for `rel`, in order.
fn candidates(dir: &Path, rel: &str) -> [PathBuf; 4] {
    [
        dir.join(format!("{rel}.luau")),
        dir.join(format!("{rel}.lua")),
        dir.join(rel).join("init.luau"),
        dir.join(rel).join("init.lua"),
    ]
}

fn require(lua: &Lua, dir: &Path, name: &str) -> mlua::Result<Value> {
    let rel = relative(name).map_err(err)?;
    let cache: Table = lua.named_registry_value(CACHE)?;
    let cached: Value = cache.get(rel.as_str())?;
    if !cached.is_nil() {
        return Ok(cached);
    }
    let loading: Table = lua.named_registry_value(LOADING)?;
    if loading.get::<bool>(rel.as_str())? {
        return Err(err(format!("require: cycle through '{name}'")));
    }
    let tried = candidates(dir, &rel);
    let Some(file) = tried.iter().find(|p| p.is_file()) else {
        let list: Vec<String> = tried.iter().map(|p| p.display().to_string()).collect();
        return Err(err(format!(
            "require: module '{name}' not found; tried {}",
            list.join(", ")
        )));
    };
    let src = std::fs::read(file).map_err(|e| err(format!("require: {}: {e}", file.display())))?;
    loading.set(rel.as_str(), true)?;
    let result = lua.load(src).set_name(name).eval::<Value>();
    loading.set(rel.as_str(), Value::Nil)?;
    let value = match result? {
        Value::Nil => Value::Boolean(true),
        v => v,
    };
    cache.set(rel.as_str(), &value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::relative;

    #[test]
    fn names_map_to_relative_paths() {
        assert_eq!(relative("keymaps"), Ok("keymaps".into()));
        assert_eq!(relative("./ui.status"), Ok("ui/status".into()));
        for bad in [
            "",
            "./",
            "..",
            "a..b",
            "/etc/passwd",
            "k.luau",
            "k.lua",
            "a.",
            ".a",
        ] {
            assert!(relative(bad).is_err(), "{bad:?}");
        }
    }
}
