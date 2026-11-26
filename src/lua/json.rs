//! `grofe.json` and the Luau/JSON conversions the store, options and
//! config share.

use mlua::{Lua, Table, Value};
use serde_json::Value as J;

/// Tables nested deeper than this are refused, which also stops a cycle.
const MAX_DEPTH: usize = 64;

fn err(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(e.to_string())
}

pub fn install(lua: &Lua, g: &Table) -> mlua::Result<()> {
    let json = lua.create_table()?;
    json.set(
        "encode",
        lua.create_function(|_, v: Value| serde_json::to_string(&to_json(&v)?).map_err(err))?,
    )?;
    json.set(
        "decode",
        lua.create_function(|lua, text: String| {
            let v: J = serde_json::from_str(&text).map_err(|e| err(format!("json.decode: {e}")))?;
            from_json(lua, &v)
        })?,
    )?;
    g.set("json", json)
}

/// A Luau value as JSON: a table holding 1..n is an array, any other table
/// an object with string keys. Functions, userdata and NaN are errors.
pub fn to_json(v: &Value) -> mlua::Result<J> {
    to_json_at(v, 0)
}

fn to_json_at(v: &Value, depth: usize) -> mlua::Result<J> {
    if depth > MAX_DEPTH {
        return Err(err("table is nested too deep (or loops)"));
    }
    Ok(match v {
        Value::Nil => J::Null,
        Value::Boolean(b) => J::Bool(*b),
        Value::Integer(i) => J::from(*i),
        Value::Number(n) => number(*n)?,
        Value::String(s) => J::String(s.to_str()?.to_string()),
        Value::Table(t) => table_to_json(t, depth)?,
        other => return Err(err(format!("a {} cannot be JSON", other.type_name()))),
    })
}

fn number(n: f64) -> mlua::Result<J> {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        return Ok(J::from(n as i64));
    }
    serde_json::Number::from_f64(n)
        .map(J::Number)
        .ok_or_else(|| err("NaN and infinity cannot be JSON"))
}

fn table_to_json(t: &Table, depth: usize) -> mlua::Result<J> {
    let len = t.raw_len();
    if len > 0 && t.pairs::<Value, Value>().count() == len {
        let items: mlua::Result<Vec<J>> = t
            .sequence_values::<Value>()
            .map(|v| to_json_at(&v?, depth + 1))
            .collect();
        return items.map(J::Array);
    }
    let mut map = serde_json::Map::new();
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let Value::String(k) = k else {
            return Err(err("an object needs string keys"));
        };
        map.insert(k.to_str()?.to_string(), to_json_at(&v, depth + 1)?);
    }
    Ok(J::Object(map))
}

pub fn from_json(lua: &Lua, v: &J) -> mlua::Result<Value> {
    Ok(match v {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Boolean(*b),
        J::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        J::String(s) => Value::String(lua.create_string(s)?),
        J::Array(items) => {
            let t = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                t.set(i + 1, from_json(lua, item)?)?;
            }
            Value::Table(t)
        }
        J::Object(map) => {
            let t = lua.create_table()?;
            for (k, item) in map {
                t.set(k.as_str(), from_json(lua, item)?)?;
            }
            Value::Table(t)
        }
    })
}

/// `over` laid on `base`: objects merge key by key, anything else replaces.
pub fn merge(base: &mut J, over: J) {
    match (base, over) {
        (J::Object(b), J::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_merge_nests_and_replaces() {
        let mut a = json!({"a": 1, "o": {"x": 1, "y": 2}, "l": [1]});
        merge(&mut a, json!({"o": {"y": 20, "z": 3}, "l": [2, 3]}));
        assert_eq!(
            a,
            json!({"a": 1, "o": {"x": 1, "y": 20, "z": 3}, "l": [2, 3]})
        );
    }
}
