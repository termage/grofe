use super::*;
use crate::lua::runtime::Limits;
use std::time::{Duration, Instant};

#[derive(Default)]
struct FakeHost {
    runs: Vec<Vec<String>>,
    reply: String,
    fail: Option<String>,
    binds: Vec<(String, String, u32, Option<String>, bool)>,
    everys: Vec<(u64, u32)>,
    cancelled: Vec<u32>,
}

impl Host for FakeHost {
    fn run(&mut self, argv: Vec<String>) -> Result<String, String> {
        self.runs.push(argv);
        match &self.fail {
            Some(e) => Err(e.clone()),
            None => Ok(self.reply.clone()),
        }
    }
    fn expand(&mut self, format: &str, _: Option<&str>) -> Result<String, String> {
        Ok(format!("<{format}>"))
    }
    fn bind(
        &mut self,
        table: &str,
        key: &str,
        id: u32,
        desc: Option<String>,
        repeat: bool,
    ) -> Result<(), String> {
        self.binds
            .push((table.into(), key.into(), id, desc, repeat));
        Ok(())
    }
    fn subscribe(&mut self, _: &str, _: u32) -> Result<(), String> {
        Ok(())
    }
    fn every(&mut self, ms: u64, id: u32) -> Result<(), String> {
        self.everys.push((ms, id));
        Ok(())
    }
    fn system(&mut self, _: Vec<String>, _: Option<String>, _: u64, _: u32) -> Result<(), String> {
        Ok(())
    }
    fn cancel(&mut self, id: u32) {
        self.cancelled.push(id);
    }
    fn notify(&mut self, _: &str, _: Option<&str>, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn log(&mut self, _: &str) {}
}

fn rt() -> Runtime {
    let rt = Runtime::new(Limits::default()).unwrap();
    install(&rt).unwrap();
    rt
}

fn eval<R: mlua::FromLuaMulti>(
    rt: &Runtime,
    host: &mut FakeHost,
    src: &str,
) -> Result<R, LuaError> {
    enter(rt, host, "t", |l| l.load(src).eval::<R>())
}

#[test]
fn test_cmd_table_reaches_host() {
    let rt = rt();
    let mut h = FakeHost {
        reply: "%3".into(),
        ..Default::default()
    };
    let out: String = eval(&rt, &mut h, "return grofe.cmd{'split-window','-P'}").unwrap();
    assert_eq!(out, "%3");
    assert_eq!(h.runs, vec![vec!["split-window", "-P"]]);
}

#[test]
fn test_cmd_string_splits_each_command() {
    let rt = rt();
    let mut h = FakeHost {
        reply: "x".into(),
        ..Default::default()
    };
    let out: String = eval(
        &rt,
        &mut h,
        "return grofe.cmd('kill-pane -t %1; new-window')",
    )
    .unwrap();
    assert_eq!(
        h.runs,
        vec![vec!["kill-pane", "-t", "%1"], vec!["new-window"]]
    );
    assert_eq!(out, "x\nx");
}

#[test]
fn test_bind_registers_and_returns_id() {
    let rt = rt();
    let mut h = FakeHost::default();
    let id: u32 = eval(
        &rt,
        &mut h,
        "return grofe.bind('prefix', 'g', function() end, {desc = 'Git'})",
    )
    .unwrap();
    assert_eq!(id, 1);
    assert_eq!(
        h.binds,
        vec![("prefix".into(), "g".into(), 1, Some("Git".into()), false)]
    );
}

#[test]
fn test_cmd_error_raises() {
    let rt = rt();
    let mut h = FakeHost {
        fail: Some("no such pane".into()),
        ..Default::default()
    };
    let msg: String = eval(
        &rt,
        &mut h,
        "local ok, e = pcall(grofe.cmd, {'kill-pane'}) return tostring(e)",
    )
    .unwrap();
    assert!(msg.contains("no such pane"), "{msg}");
}

#[test]
fn test_call_unknown_id_is_ok() {
    let rt = rt();
    let mut h = FakeHost::default();
    call(&rt, &mut h, 99, Payload::None).unwrap();
    eval::<()>(
        &rt,
        &mut h,
        "local id = grofe.every(500, function() end) grofe.cancel(id)",
    )
    .unwrap();
    call(&rt, &mut h, 1, Payload::None).unwrap();
    assert_eq!(h.cancelled, vec![1]);
}

#[test]
fn test_grofe_outside_enter_errors() {
    let rt = rt();
    let e = rt.exec("o", "grofe.cmd{'x'}").unwrap_err();
    assert!(e.message.contains("grofe.cmd outside a grofe call"), "{e}");
}

#[test]
fn test_every_below_100ms_clamps() {
    let rt = rt();
    let mut h = FakeHost::default();
    eval::<()>(&rt, &mut h, "grofe.every(5, function() end)").unwrap();
    assert_eq!(h.everys, vec![(100, 1)]);
}

#[test]
fn test_api_is_1() {
    let rt = rt();
    assert_eq!(rt.lua().load("return grofe.api").eval::<u32>().unwrap(), 1);
}

#[test]
fn test_call_passes_payloads() {
    let rt = rt();
    let mut h = FakeHost::default();
    eval::<()>(&rt, &mut h, "grofe.on('e', function(p) seen = p.a[1] end) grofe.system({'x'}, function(r) code = r.code end)").unwrap();
    call(
        &rt,
        &mut h,
        1,
        Payload::Event(serde_json::json!({"a": [7]})),
    )
    .unwrap();
    call(
        &rt,
        &mut h,
        2,
        Payload::System {
            code: 3,
            stdout: String::new(),
            stderr: String::new(),
        },
    )
    .unwrap();
    let g = rt.lua().globals();
    assert_eq!(
        (g.get::<i64>("seen").unwrap(), g.get::<i64>("code").unwrap()),
        (7, 3)
    );
}

#[test]
fn test_plugin_value_round_trips() {
    let rt = rt();
    set_plugin(&rt, "p", Value::Integer(5)).unwrap();
    let mut h = FakeHost::default();
    assert_eq!(
        eval::<i64>(&rt, &mut h, "return grofe.plugin('p')").unwrap(),
        5
    );
    assert!(
        eval::<Value>(&rt, &mut h, "return grofe.plugin('q')")
            .unwrap()
            .is_nil()
    );
}

#[test]
fn test_callback_over_budget_errors() {
    let rt = rt();
    let mut h = FakeHost::default();
    eval::<()>(
        &rt,
        &mut h,
        "grofe.bind('root', 'x', function() while true do end end)",
    )
    .unwrap();
    let t = Instant::now();
    let e = call(&rt, &mut h, 1, Payload::None).unwrap_err();
    assert!(
        e.who.contains("callback 1") && e.message.contains("budget"),
        "{e}"
    );
    assert!(t.elapsed() < Duration::from_millis(150));
}
