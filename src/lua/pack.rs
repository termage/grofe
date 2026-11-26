//! What Luau code knows about the pack it runs for: the folder, the
//! options, a durable store, and which pack owns each callback.

use mlua::{Function, Lua, MultiValue, Table, Value};
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

/// Makes `name` known to Luau with its folder and merged options, and
/// builds the environment its chunk and modules run in.
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
    info.set("env", environment(lua, name)?)?;
    lua.named_registry_value::<Table>(INFO)?.set(name, info)
}

/// What a pack's code sees as globals: `require` and the `grofe` calls that
/// depend on the pack, bound to `name` by where the code is written, so a
/// pack function called from `init.luau` is still the pack's.
fn environment(lua: &Lua, name: &str) -> mlua::Result<Table> {
    let globals = lua.globals();
    let real: Table = globals.get("grofe")?;
    let proxy = lua.create_table()?;
    for f in ["bind", "on", "every", "system", "pack_dir", "options"] {
        proxy.set(f, bound(lua, name, real.get(f)?)?)?;
    }
    let (store, real_store) = (lua.create_table()?, real.get::<Table>("store")?);
    for f in ["get", "set", "delete"] {
        store.set(f, bound(lua, name, real_store.get(f)?)?)?;
    }
    proxy.set("store", store)?;
    let to = |t: &Table| -> mlua::Result<Table> {
        let meta = lua.create_table()?;
        meta.set("__index", t)?;
        Ok(meta)
    };
    proxy.set_metatable(Some(to(&real)?))?;
    let env = lua.create_table()?;
    env.set("grofe", proxy)?;
    env.set("require", bound(lua, name, globals.get("require")?)?)?;
    env.set_metatable(Some(to(&globals)?))?;
    Ok(env)
}

/// `f`, run as pack `name`.
fn bound(lua: &Lua, name: &str, f: Function) -> mlua::Result<Function> {
    let name = name.to_string();
    lua.create_function(move |lua, args: MultiValue| {
        let before = current(lua)?;
        set_current(lua, Some(&name))?;
        let result = f.call::<MultiValue>(args);
        set_current(lua, before.as_deref())?;
        result
    })
}

/// The environment of pack `name`, for its chunk.
pub fn env_of(lua: &Lua, name: &str) -> mlua::Result<Table> {
    lua.named_registry_value::<Table>(INFO)?
        .get::<Table>(name)?
        .get("env")
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

/// The environment of the running pack, for the modules it requires.
pub fn current_env(lua: &Lua) -> mlua::Result<Option<Table>> {
    match current(lua)? {
        Some(name) => env_of(lua, &name).map(Some),
        None => Ok(None),
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
            let value = to_json(&value)?;
            update_store(&store_path(lua)?, |map| {
                match value {
                    serde_json::Value::Null => map.remove(&key),
                    v => map.insert(key, v),
                };
            })
        })?,
    )?;
    t.set(
        "delete",
        lua.create_function(|lua, key: String| {
            update_store(&store_path(lua)?, |map| {
                map.remove(&key);
            })
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

/// Applies `change` to the store under an exclusive file lock, so servers
/// sharing the state directory do not lose each other's writes.
pub(crate) fn update_store(path: &Path, change: impl FnOnce(&mut StoreMap)) -> mlua::Result<()> {
    let fail = |e: std::io::Error| err(format!("store {}: {e}", path.display()));
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(fail)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(fail)?;
    // Released when `_guard` drops.
    let _guard = nix::fcntl::Flock::lock(lock, nix::fcntl::FlockArg::LockExclusive)
        .map_err(|(_, e)| fail(e.into()))?;
    let mut map = read_store(path)?;
    change(&mut map);
    let text = serde_json::to_string(&map).map_err(err)?;
    write_durably(path, &text).map_err(fail)
}

/// Writes to a uniquely named file, syncs it, renames it over `path` and
/// syncs the folder, so a crash keeps either the old store or the new one.
fn write_durably(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let tmp = path.with_extension(format!("{}.{nanos}.tmp", std::process::id()));
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)?;
    match path.parent() {
        Some(dir) => std::fs::File::open(dir)?.sync_all(),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_concurrent_updates_lose_nothing() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("p/store.json");
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        update_store(&path, |m| {
                            let n = m.get("n").and_then(|v| v.as_i64()).unwrap_or(0);
                            m.insert("n".into(), (n + 1).into());
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().unwrap();
        }
        assert_eq!(read_store(&path).unwrap()["n"], 100);
        let leftovers = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }
}
