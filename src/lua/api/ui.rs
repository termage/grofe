//! The Lua side of the UI bindings: `grofe.context`, `grofe.choose` and
//! `grofe.input`, and the jobs they hand the host.

use super::{err, host_fn};
use crate::lua::callbacks::add_callback;
use mlua::{Function, Lua, Table, Value};

/// What `grofe.context()` reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    pub client: Option<String>,
    pub pane: Option<String>,
    pub cwd: Option<String>,
}

/// What `grofe.choose` asks for: row labels, and the text shown for each
/// row's preview (empty without one).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChooseJob {
    pub title: Option<String>,
    pub labels: Vec<String>,
    pub previews: Vec<String>,
}

impl ChooseJob {
    fn to_table(&self, lua: &Lua) -> mlua::Result<Table> {
        let t = lua.create_table()?;
        t.set("title", self.title.as_deref())?;
        t.set("labels", self.labels.clone())?;
        t.set("previews", self.previews.clone())?;
        Ok(t)
    }

    pub(super) fn from_table(t: &Table) -> mlua::Result<ChooseJob> {
        Ok(ChooseJob {
            title: t.get("title")?,
            labels: t.get("labels")?,
            previews: t.get("previews")?,
        })
    }
}

/// The labels, previews and values of `grofe.choose`'s `items` (strings, or
/// `{label, value}` tables); `preview` is called on each item. The values are
/// kept in a table by 1-based row.
fn parse_items(
    lua: &Lua,
    items: Table,
    title: Option<String>,
    preview: Option<Function>,
) -> mlua::Result<(ChooseJob, Table)> {
    let mut job = ChooseJob {
        title,
        ..ChooseJob::default()
    };
    let values = lua.create_table()?;
    for (n, item) in items.sequence_values::<Value>().enumerate() {
        let item = item?;
        let (label, value) = match &item {
            Value::String(s) => (s.to_str()?.to_string(), item.clone()),
            Value::Table(t) => {
                let label = t.get::<Option<String>>("label")?;
                let label = label.ok_or_else(|| {
                    err(format!("grofe.choose: item {} has no string label", n + 1))
                })?;
                // No `value` gives the label itself, so `fn(nil)` only means cancel.
                let value = match t.get::<Value>("value")? {
                    Value::Nil => Value::String(lua.create_string(&label)?),
                    v => v,
                };
                (label, value)
            }
            _ => {
                return Err(err(format!(
                    "grofe.choose: item {} is not a string or a {{label, value}} table",
                    n + 1
                )));
            }
        };
        if let Some(p) = &preview {
            let text = p.call::<Option<String>>(item)?;
            job.previews.push(text.unwrap_or_default());
        }
        job.labels.push(label);
        values.set(n + 1, value)?;
    }
    Ok((job, values))
}

pub(super) fn install(lua: &Lua, g: &Table) -> mlua::Result<()> {
    g.set(
        "context",
        lua.create_function(|lua, ()| {
            let (client, pane, cwd) =
                host_fn(lua, "context")?
                    .call::<(Option<String>, Option<String>, Option<String>)>(())?;
            let t = lua.create_table()?;
            t.set("client", client)?;
            t.set("pane", pane)?;
            t.set("cwd", cwd)?;
            Ok(t)
        })?,
    )?;
    g.set(
        "choose",
        lua.create_function(|lua, (items, f, opts): (Value, Value, Option<Table>)| {
            let (Value::Table(items), Value::Function(f)) = (items, f) else {
                return Err(err("grofe.choose takes a list of items and a function"));
            };
            let (title, preview) = match opts {
                Some(o) => (
                    o.get::<Option<String>>("title")?,
                    o.get::<Value>("preview")?,
                ),
                None => (None, Value::Nil),
            };
            let preview = match preview {
                Value::Nil => None,
                Value::Function(p) => Some(p),
                _ => return Err(err("grofe.choose: preview must be a function")),
            };
            let (job, values) = parse_items(lua, items, title, preview)?;
            // An empty list has nothing to choose from: it answers at once.
            if job.labels.is_empty() {
                return f.call::<()>(Value::Nil);
            }
            let host = host_fn(lua, "choose")?;
            let pick = lua.create_function(move |_, pick: Option<usize>| match pick {
                Some(i) => f.call::<()>(values.get::<Value>(i)?),
                None => f.call::<()>(Value::Nil),
            })?;
            add_callback(lua, pick, |id| host.call((job.to_table(lua)?, id)))?;
            Ok(())
        })?,
    )?;
    g.set(
        "input",
        lua.create_function(|lua, (prompt, f): (Value, Value)| {
            let (Value::String(prompt), Value::Function(f)) = (prompt, f) else {
                return Err(err("grofe.input takes a prompt string and a function"));
            };
            let host = host_fn(lua, "input")?;
            let prompt = prompt.to_str()?.to_string();
            add_callback(lua, f, |id| host.call((prompt, id)))?;
            Ok(())
        })?,
    )?;
    Ok(())
}
