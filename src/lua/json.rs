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
        super::runtime::host_function(lua, |_, v: Value| {
            serde_json::to_string(&to_json(&v)?).map_err(err)
        })?,
    )?;
    json.set(
        "decode",
        super::runtime::host_function(lua, |lua, text: String| {
            let v: J = serde_json::from_str(&text).map_err(|e| err(format!("json.decode: {e}")))?;
            from_json(lua, &v)
        })?,
    )?;
    g.set("json", json)
}

/// A Luau value as JSON: a table holding 1..n is an array, any other table
/// an object with string keys. Functions, userdata and NaN are errors.
pub fn to_json(v: &Value) -> mlua::Result<J> {
    let mut budget = MAX_NODES;
    to_json_at(v, 0, &mut budget)
}

/// A table shared many times is walked once per reference, so the walk is
/// capped by the values it visits, not just by depth.
const MAX_NODES: usize = 100_000;

fn to_json_at(v: &Value, depth: usize, budget: &mut usize) -> mlua::Result<J> {
    if depth > MAX_DEPTH {
        return Err(err("table is nested too deep (or loops)"));
    }
    *budget = budget
        .checked_sub(1)
        .ok_or_else(|| err("table is too big to be JSON"))?;
    Ok(match v {
        Value::Nil => J::Null,
        Value::Boolean(b) => J::Bool(*b),
        Value::Integer(i) => J::from(*i),
        Value::Number(n) => number(*n)?,
        Value::String(s) => J::String(s.to_str()?.to_string()),
        Value::Table(t) => table_to_json(t, depth, budget)?,
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

/// Longest array a sparse table may make, holes included.
const MAX_ARRAY: usize = 100_000;

/// A table whose keys are all positive integers is an array (holes are
/// null), one with string keys an object; an empty table is `{}`.
fn table_to_json(t: &Table, depth: usize, budget: &mut usize) -> mlua::Result<J> {
    let mut items: Vec<(usize, J)> = Vec::new();
    let mut map = serde_json::Map::new();
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let v = to_json_at(&v, depth + 1, budget)?;
        match k {
            Value::String(k) => {
                map.insert(k.to_str()?.to_string(), v);
            }
            Value::Integer(i) if i >= 1 => items.push((i as usize, v)),
            Value::Number(n) if n >= 1.0 && n.fract() == 0.0 => items.push((n as usize, v)),
            _ => return Err(err("a table needs string keys or positive integer keys")),
        }
    }
    if !items.is_empty() && !map.is_empty() {
        return Err(err("a table cannot mix string and integer keys"));
    }
    if items.is_empty() {
        return Ok(J::Object(map));
    }
    let len = items.iter().map(|(i, _)| *i).max().unwrap_or(0);
    if len > MAX_ARRAY {
        return Err(err("a sparse table is too long to be an array"));
    }
    let mut array = vec![J::Null; len];
    for (i, v) in items {
        array[i - 1] = v;
    }
    Ok(J::Array(array))
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

/// A parsed TOML value as JSON. A datetime is its text; a number JSON
/// cannot hold (beyond `i64`, infinite, NaN) is an error, not null.
pub fn toml_to_json(v: &toml::Value) -> Result<J, String> {
    use toml::Value as T;
    Ok(match v {
        T::String(s) => J::String(s.clone()),
        T::Integer(i) => J::from(*i),
        T::Float(f) => float(*f)?,
        T::Boolean(b) => J::Bool(*b),
        T::Datetime(d) => J::String(d.to_string()),
        T::Array(items) => J::Array(items.iter().map(toml_to_json).collect::<Result<_, _>>()?),
        T::Table(t) => J::Object(
            t.iter()
                .map(|(k, v)| Ok((k.clone(), toml_to_json(v)?)))
                .collect::<Result<_, String>>()?,
        ),
    })
}

/// [`toml_to_json`] for a value still carrying its source spans.
pub fn de_to_json(v: &toml::de::DeValue) -> Result<J, String> {
    use toml::de::DeValue as D;
    Ok(match v {
        D::String(s) => J::String(s.to_string()),
        D::Integer(i) => i64::from_str_radix(i.as_str(), i.radix())
            .map(J::from)
            .map_err(|_| format!("{} does not fit in 64 bits", i.as_str()))?,
        D::Float(f) => float(f.as_str().parse::<f64>().map_err(|e| e.to_string())?)?,
        D::Boolean(b) => J::Bool(*b),
        D::Datetime(d) => J::String(d.to_string()),
        D::Array(items) => J::Array(
            items
                .iter()
                .map(|i| de_to_json(i.get_ref()))
                .collect::<Result<_, _>>()?,
        ),
        D::Table(t) => J::Object(
            t.iter()
                .map(|(k, v)| Ok((k.get_ref().to_string(), de_to_json(v.get_ref())?)))
                .collect::<Result<_, String>>()?,
        ),
    })
}

fn float(f: f64) -> Result<J, String> {
    serde_json::Number::from_f64(f)
        .map(J::Number)
        .ok_or_else(|| format!("{f} cannot be used as a number here"))
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
    fn test_toml_converts_datetimes_and_refuses_unrepresentable_numbers() {
        let t: toml::Value = toml::from_str("d = 1979-05-27\nn = 3\n[t]\nf = 1.5").unwrap();
        assert_eq!(
            toml_to_json(&t),
            Ok(json!({"d": "1979-05-27", "n": 3, "t": {"f": 1.5}}))
        );
        let inf: toml::Value = toml::from_str("x = inf").unwrap();
        assert!(toml_to_json(&inf).is_err());
        let de = toml::de::DeTable::parse("x = nan\n").unwrap();
        assert!(de_to_json(&toml::de::DeValue::Table(de.into_inner())).is_err());
        let de = toml::de::DeTable::parse("x = 0xFFFFFFFFFFFFFFFFF\n").unwrap();
        assert!(de_to_json(&toml::de::DeValue::Table(de.into_inner())).is_err());
    }

    #[test]
    fn test_a_table_shared_many_times_is_refused_quickly() {
        let lua = Lua::new();
        let t: Value = lua
            .load("local t = {} for i = 1, 40 do t = {t, t} end return t")
            .eval()
            .unwrap();
        let start = std::time::Instant::now();
        assert!(to_json(&t).is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn test_holes_are_null_and_round_trip() {
        let lua = Lua::new();
        let t: Value = lua.load("return {1, nil, 3}").eval().unwrap();
        assert_eq!(to_json(&t).unwrap(), json!([1, null, 3]));
        let back = from_json(&lua, &json!([1, null, 3])).unwrap();
        assert_eq!(to_json(&back).unwrap(), json!([1, null, 3]));
    }

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
