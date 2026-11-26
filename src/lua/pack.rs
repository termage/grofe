//! What Luau code knows about the pack it runs for: the folder, the
//! options, a durable store, and which pack owns each callback.

use mlua::{Lua, Table, Value};
use std::path::{Path, PathBuf};

use super::json::{from_json, to_json};
use crate::paths::{Env, state_dir};

/// The name of the pack whose code is running, or nil.
const CURRENT: &str = "grofe_pack_current";
/// Pack name to `{ dir, options }`.
const INFO: &str = "grofe_pack_info";
/// Callback id to the name of the pack that made it.
const OWNERS: &str = "grofe_pack_owners";

fn err(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(e.to_string())
}

pub fn install(lua: &Lua, g: &Table) -> mlua::Result<()> {
    lua.set_named_registry_value(INFO, lua.create_table()?)?;
    lua.set_named_registry_value(OWNERS, lua.create_table()?)?;
    g.set(
        "pack_dir",
        lua.create_function(|lua, ()| {
            let dir = current_dir(lua)?.ok_or_else(|| err("grofe.pack_dir outside a pack"))?;
            Ok(dir.to_string_lossy().into_owned())
        })?,
    )?;
    g.set(
        "options",
        lua.create_function(|lua, ()| {
            let name = current(lua)?.ok_or_else(|| err("grofe.options outside a pack"))?;
            lua.named_registry_value::<Table>(INFO)?
                .get::<Table>(name)?
                .get::<Value>("options")
        })?,
    )?;
    g.set("store", store_table(lua)?)
}

/// Makes `name` known to Luau with its folder and merged options.
pub fn register(
    lua: &Lua,
    name: &str,
    dir: &Path,
    options: &serde_json::Value,
) -> mlua::Result<()> {
    let info = lua.create_table()?;
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    info.set("dir", dir.to_string_lossy().into_owned())?;
    info.set("options", from_json(lua, options)?)?;
    lua.named_registry_value::<Table>(INFO)?.set(name, info)
}

pub fn set_current(lua: &Lua, name: Option<&str>) -> mlua::Result<()> {
    lua.set_named_registry_value(CURRENT, name)
}

fn current(lua: &Lua) -> mlua::Result<Option<String>> {
    lua.named_registry_value(CURRENT)
}

/// The folder of the running pack, for `require` and `grofe.pack_dir`.
pub fn current_dir(lua: &Lua) -> mlua::Result<Option<PathBuf>> {
    let Some(name) = current(lua)? else {
        return Ok(None);
    };
    let info: Value = lua.named_registry_value::<Table>(INFO)?.get(name)?;
    match info {
        Value::Table(t) => Ok(Some(PathBuf::from(t.get::<String>("dir")?))),
        _ => Ok(None),
    }
}

/// Records the running pack as the owner of callback `id`.
pub fn claim(lua: &Lua, id: u32) -> mlua::Result<()> {
    lua.named_registry_value::<Table>(OWNERS)?
        .set(id, current(lua)?)
}

pub fn owner(lua: &Lua, id: u32) -> mlua::Result<Option<String>> {
    lua.named_registry_value::<Table>(OWNERS)?.get(id)
}

fn store_table(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "get",
        lua.create_function(|lua, key: String| {
            let value = read_store(&store_path(lua)?)?.remove(&key);
            value.map_or(Ok(Value::Nil), |v| from_json(lua, &v))
        })?,
    )?;
    t.set(
        "set",
        lua.create_function(|lua, (key, value): (String, Value)| {
            let path = store_path(lua)?;
            let mut map = read_store(&path)?;
            match to_json(&value)? {
                serde_json::Value::Null => map.remove(&key),
                v => map.insert(key, v),
            };
            write_store(&path, &map)
        })?,
    )?;
    t.set(
        "delete",
        lua.create_function(|lua, key: String| {
            let path = store_path(lua)?;
            let mut map = read_store(&path)?;
            if map.remove(&key).is_some() {
                write_store(&path, &map)?;
            }
            Ok(())
        })?,
    )?;
    Ok(t)
}

type StoreMap = serde_json::Map<String, serde_json::Value>;

fn store_path(lua: &Lua) -> mlua::Result<PathBuf> {
    let name = current(lua)?.ok_or_else(|| err("grofe.store outside a pack"))?;
    let root = state_dir(&Env::from_process());
    Ok(root.join("packs").join(name).join("store.json"))
}

fn read_store(path: &Path) -> mlua::Result<StoreMap> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|e| err(format!("store {}: {e}", path.display())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(StoreMap::new()),
        Err(e) => Err(err(format!("store {}: {e}", path.display()))),
    }
}

/// Writes beside the file and renames, so a crash keeps the old store.
fn write_store(path: &Path, map: &StoreMap) -> mlua::Result<()> {
    let text = serde_json::to_string(map).map_err(err)?;
    let tmp = path.with_extension("json.tmp");
    let put = || -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)
    };
    put().map_err(|e| err(format!("store {}: {e}", path.display())))
}
