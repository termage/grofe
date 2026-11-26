//! `require` for the user's config folder: a Rust function that replaces
//! mlua's global `require`, so `init.luau` can split itself into modules.

use super::pack;
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
fn err(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(e.to_string())
}

/// The config folder `set_dir` pointed `require` at.
pub fn config_dir(lua: &Lua) -> mlua::Result<Option<PathBuf>> {
    match lua.named_registry_value::<Value>(DIR)? {
        Value::String(dir) => Ok(Some(PathBuf::from(OsStr::from_bytes(&dir.as_bytes())))),
        _ => Ok(None),
    }
}

/// Replaces the global `require`. Call before `sandbox(true)`. Code a pack
/// runs loads from the pack's folder, any other from the config folder.
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let f = lua.create_function(|lua, name: String| {
        let dir = match pack::current_dir(lua)? {
            Some(dir) => dir,
            None => config_dir(lua)?
                .ok_or_else(|| err(format!("require: no config folder to load '{name}' from")))?,
        };
        require(lua, &dir, &name)
    })?;
    lua.globals().set("require", f)
}

/// Points `require` at `dir` and starts an empty module cache, unless it
/// already points there (packs and `init.luau` share one load's cache).
pub fn set_dir(lua: &Lua, dir: &Path) -> mlua::Result<()> {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let bytes = dir.as_os_str().as_bytes();
    if let Value::String(cur) = lua.named_registry_value::<Value>(DIR)?
        && *cur.as_bytes() == *bytes
    {
        return Ok(());
    }
    lua.set_named_registry_value(DIR, lua.create_string(bytes)?)?;
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
    // One module name in two folders is two modules.
    let key = dir.join(&rel).to_string_lossy().into_owned();
    let cache: Table = lua.named_registry_value(CACHE)?;
    let cached: Value = cache.get(key.as_str())?;
    if !cached.is_nil() {
        return Ok(cached);
    }
    let loading: Table = lua.named_registry_value(LOADING)?;
    if loading.get::<bool>(key.as_str())? {
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
    loading.set(key.as_str(), true)?;
    let result = lua.load(src).set_name(name).eval::<Value>();
    loading.set(key.as_str(), Value::Nil)?;
    let value = match result? {
        Value::Nil => Value::Boolean(true),
        v => v,
    };
    cache.set(key.as_str(), &value)?;
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
