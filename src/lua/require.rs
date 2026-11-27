//! `require` for the user's config folder: a Rust function that replaces
//! mlua's global `require`, so `init.luau` can split itself into modules.

use super::pack;
use mlua::{Function, Lua, Table, Value};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The canonical config folder, set by the init load.
const DIR: &str = "grofe_require_dir";
/// Module name to the value it returned, for this load.
const CACHE: &str = "grofe_require_cache";
/// Modules whose chunk is running, to catch cycles.
const LOADING: &str = "grofe_require_loading";
/// Module key, then environment (`false` for none), to its source and what
/// compiling that gave (`f` or `err`), for the runtime's life: compiling is
/// off the budget, so a module required again (after it raised, say) must
/// not be compiled again.
const COMPILED: &str = "grofe_require_compiled";
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
        let (dir, env) = match pack::current_dir(lua)? {
            Some(dir) => (dir, pack::current_env(lua)?),
            None => {
                let dir = config_dir(lua)?.ok_or_else(|| {
                    err(format!("require: no config folder to load '{name}' from"))
                })?;
                (dir, None)
            }
        };
        require(lua, &dir, &name, env)
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

fn require(lua: &Lua, dir: &Path, name: &str, env: Option<Table>) -> mlua::Result<Value> {
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
    let f = compiled(lua, &key, &src, name, env)?;
    loading.set(key.as_str(), true)?;
    let result = f.call::<Value>(());
    loading.set(key.as_str(), Value::Nil)?;
    let value = match result? {
        Value::Nil => Value::Boolean(true),
        v => v,
    };
    cache.set(key.as_str(), &value)?;
    Ok(value)
}

/// `src` compiled once per runtime under `key` (again only if it changed).
fn compiled(
    lua: &Lua,
    key: &str,
    src: &[u8],
    name: &str,
    env: Option<Table>,
) -> mlua::Result<Function> {
    let all = match lua.named_registry_value::<Option<Table>>(COMPILED)? {
        Some(t) => t,
        None => {
            let t = lua.create_table()?;
            lua.set_named_registry_value(COMPILED, &t)?;
            t
        }
    };
    // One file reached from the config folder and from a pack's runs in
    // two environments, and a function is compiled for one: one entry each.
    let per_env = match all.get::<Option<Table>>(key)? {
        Some(t) => t,
        None => {
            let t = lua.create_table()?;
            all.set(key, &t)?;
            t
        }
    };
    let env_key = env.clone().map_or(Value::Boolean(false), Value::Table);
    let entry = match per_env.get::<Option<Table>>(&env_key)? {
        Some(e) if *e.get::<mlua::LuaString>("src")?.as_bytes() == *src => e,
        _ => {
            let e = lua.create_table()?;
            e.set("src", lua.create_string(src)?)?;
            let compiled = super::runtime::compile_eval(lua, src, name, env);
            match &compiled {
                Ok(f) => e.set("f", f)?,
                Err(error) => e.set("err", Value::Error(Box::new(error.clone())))?,
            }
            per_env.set(env_key, &e)?;
            return compiled;
        }
    };
    match entry.get::<Value>("f")? {
        Value::Function(f) => Ok(f),
        _ => match entry.get::<Value>("err")? {
            Value::Error(e) => Err(*e),
            other => Err(err(format!("require: no compiled '{name}': {other:?}"))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{compiled, relative};

    #[test]
    fn a_module_compiles_once_until_it_changes() {
        // Compiling is off the budget: compiled anew each time, a
        // `pcall(require, ...)` loop on a raising module would barely spend it.
        let lua = mlua::Lua::new();
        let get = |src: &str| compiled(&lua, "k", src.as_bytes(), "m", None);
        let first = get("error('x')").unwrap();
        assert_eq!(get("error('x')").unwrap(), first);
        assert_ne!(get("error('y')").unwrap(), first);
        let bad = get("x =").unwrap_err().to_string();
        assert_eq!(get("x =").unwrap_err().to_string(), bad);
    }

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
