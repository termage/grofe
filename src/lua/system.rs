//! `grofe.system`'s options, read and checked when it is called, and the
//! request that goes to the host.

use mlua::{Function, Lua, Table, Value};

pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;
/// The longest a job may run.
pub const MAX_TIMEOUT_MS: u64 = 3_600_000;

/// `opts.agent`: the job shows as a `working` row of `kind` called `label`
/// while it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRecord {
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemJob {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub timeout_ms: u64,
    pub env: Vec<(String, String)>,
    pub stdin: Option<String>,
    /// The callback id of `on_line`.
    pub on_line: Option<u32>,
    pub agent: Option<AgentRecord>,
}

fn err(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(e.to_string())
}

/// The keys `grofe.system` takes in its options table.
const OPTS: [&str; 6] = ["cwd", "env", "stdin", "on_line", "timeout_ms", "agent"];

fn agent_of(t: Table) -> mlua::Result<AgentRecord> {
    for pair in t.pairs::<Value, Value>() {
        let (k, _) = pair?;
        match &k {
            Value::String(s) if ["kind", "label"].contains(&&*s.to_str()?) => {}
            other => {
                return Err(err(format!(
                    "grofe.system: unknown agent field {}; fields are kind, label",
                    other.to_string()?
                )));
            }
        }
    }
    let need = |name: &str| {
        t.get::<Option<String>>(name)?
            .filter(|v| !v.is_empty())
            .ok_or_else(|| err(format!("grofe.system: agent.{name} is required")))
    };
    Ok(AgentRecord {
        kind: need("kind")?,
        label: need("label")?,
    })
}

/// The job `argv` and `opts` ask for, and its `on_line` function.
pub fn read(argv: Vec<String>, opts: Option<Table>) -> mlua::Result<(SystemJob, Option<Function>)> {
    let mut job = SystemJob {
        argv,
        cwd: None,
        timeout_ms: DEFAULT_TIMEOUT_MS,
        env: Vec::new(),
        stdin: None,
        on_line: None,
        agent: None,
    };
    let Some(o) = opts else {
        return Ok((job, None));
    };
    for pair in o.pairs::<Value, Value>() {
        let (k, _) = pair?;
        match &k {
            Value::String(s) if OPTS.contains(&&*s.to_str()?) => {}
            other => {
                return Err(err(format!(
                    "grofe.system: unknown option {}; options are {}",
                    other.to_string()?,
                    OPTS.join(", ")
                )));
            }
        }
    }
    job.cwd = opt_get(&o, "cwd")?;
    job.stdin = opt_get(&o, "stdin")?;
    if let Some(ms) = opt_get::<Option<u64>>(&o, "timeout_ms")? {
        if ms > MAX_TIMEOUT_MS {
            return Err(err(format!(
                "grofe.system: timeout_ms is at most {MAX_TIMEOUT_MS} (one hour)"
            )));
        }
        job.timeout_ms = ms;
    }
    if let Some(env) = opt_get::<Option<Table>>(&o, "env")? {
        for pair in env.pairs::<String, String>() {
            let (k, v) = pair.map_err(|e| err(format!("grofe.system: env: {e}")))?;
            job.env.push((k, v));
        }
    }
    if let Some(agent) = opt_get::<Option<Table>>(&o, "agent")? {
        job.agent = Some(agent_of(agent)?);
    }
    Ok((job, opt_get(&o, "on_line")?))
}

/// An option's value, a wrong type erroring as `grofe.system: <key>: ...`.
fn opt_get<T: mlua::FromLua>(o: &Table, key: &str) -> mlua::Result<T> {
    o.get(key)
        .map_err(|e| err(format!("grofe.system: {key}: {e}")))
}

impl SystemJob {
    /// The job as a table, to cross into the scoped host functions.
    pub fn to_table(&self, lua: &Lua) -> mlua::Result<Table> {
        let t = lua.create_table()?;
        t.set("argv", self.argv.clone())?;
        t.set("cwd", self.cwd.clone())?;
        t.set("timeout_ms", self.timeout_ms)?;
        let env = lua.create_table()?;
        for (k, v) in &self.env {
            env.set(k.as_str(), v.as_str())?;
        }
        t.set("env", env)?;
        t.set("stdin", self.stdin.clone())?;
        t.set("on_line", self.on_line)?;
        if let Some(a) = &self.agent {
            let agent = lua.create_table()?;
            agent.set("kind", a.kind.clone())?;
            agent.set("label", a.label.clone())?;
            t.set("agent", agent)?;
        }
        Ok(t)
    }

    pub fn from_table(t: &Table) -> mlua::Result<SystemJob> {
        let mut env = Vec::new();
        for pair in t.get::<Table>("env")?.pairs::<String, String>() {
            env.push(pair?);
        }
        Ok(SystemJob {
            argv: t.get("argv")?,
            cwd: t.get("cwd")?,
            timeout_ms: t.get("timeout_ms")?,
            env,
            stdin: t.get("stdin")?,
            on_line: t.get("on_line")?,
            agent: t.get::<Option<Table>>("agent")?.map(agent_of).transpose()?,
        })
    }
}
