use super::*;
use crate::lua::runtime::Limits;
use std::time::{Duration, Instant};

#[derive(Default)]
struct FakeHost {
    context: Option<Context>,
    runs: Vec<Vec<String>>,
    reply: String,
    fail: Option<String>,
    binds: Vec<(String, String, u32, Option<String>, bool)>,
    everys: Vec<(u64, u32)>,
    cancelled: Vec<u32>,
    subs: Vec<(u32, Option<String>)>,
    jobs: Vec<SystemJob>,
    chooses: Vec<(ChooseJob, u32)>,
    inputs: Vec<(String, u32)>,
    notified: Vec<bool>,
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
    fn subscribe(&mut self, _: &str, id: u32, pane: Option<&str>) -> Result<(), String> {
        self.subs.push((id, pane.map(String::from)));
        Ok(())
    }
    fn every(&mut self, ms: u64, id: u32) -> Result<(), String> {
        self.everys.push((ms, id));
        Ok(())
    }
    fn system(&mut self, job: SystemJob, _: u32) -> Result<(), String> {
        self.jobs.push(job);
        Ok(())
    }
    fn agent_read(&mut self, _: &str, _: usize) -> Result<String, String> {
        Ok(String::new())
    }

    fn agent_prompt(&mut self, _: PromptJob, _: u32) -> Result<(), String> {
        Ok(())
    }

    fn agent_history(&mut self, pane: &str, n: usize) -> Result<Vec<String>, String> {
        Ok(vec![format!("{pane}:{n}")])
    }
    fn cancel(&mut self, id: u32) {
        self.cancelled.push(id);
    }
    fn notify(&mut self, _: &str, _: Option<&str>, _: &str, reply: bool) -> Result<(), String> {
        self.notified.push(reply);
        Ok(())
    }
    fn context(&mut self) -> Context {
        self.context.clone().unwrap_or_else(|| Context {
            client: Some("client-1".into()),
            pane: None,
            cwd: Some("/w".into()),
        })
    }
    fn choose(&mut self, req: ChooseJob, id: u32) -> Result<(), String> {
        self.chooses.push((req, id));
        Ok(())
    }
    fn input(&mut self, prompt: &str, id: u32) -> Result<(), String> {
        self.inputs.push((prompt.into(), id));
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

#[test]
fn test_system_options_reach_the_host() {
    let rt = rt();
    let mut h = FakeHost::default();
    let _: () = eval(
        &rt,
        &mut h,
        "grofe.system({'cat'}, function() end, {env = {A = 'b'}, stdin = 'in', \
         on_line = function() end, timeout_ms = 3600000, \
         agent = {kind = 'triage', label = 'x'}})",
    )
    .unwrap();
    let job = &h.jobs[0];
    assert_eq!(job.env, [("A".to_string(), "b".to_string())]);
    assert_eq!(job.stdin.as_deref(), Some("in"));
    assert_eq!(job.timeout_ms, 3_600_000);
    assert!(job.on_line.is_some());
    let agent = job.agent.as_ref().unwrap();
    assert_eq!((&*agent.kind, &*agent.label), ("triage", "x"));
}

#[test]
fn test_system_option_errors_raise() {
    let rt = rt();
    for (opts, want) in [
        ("{timeout_ms = 3600001}", "timeout_ms"),
        ("{agent = {label = 'x'}}", "agent.kind"),
        ("{agent = {kind = 'k'}}", "agent.label"),
        ("{agent = {kind = 'k', label = 'x', color = 1}}", "color"),
    ] {
        let mut h = FakeHost::default();
        let e = eval::<()>(
            &rt,
            &mut h,
            &format!("grofe.system({{'true'}}, function() end, {opts})"),
        )
        .unwrap_err();
        assert!(e.to_string().contains(want), "{opts}: {e}");
        assert!(h.jobs.is_empty());
    }
}

#[test]
fn test_on_takes_a_pane_scope_and_history_passes_through() {
    let rt = rt();
    let mut h = FakeHost::default();
    let out: String = eval(
        &rt,
        &mut h,
        "grofe.on('agent-changed', function() end, {pane = '%4'}) \
         return grofe.agent.history('%4', 2)[1]",
    )
    .unwrap();
    assert_eq!(out, "%4:2");
    assert_eq!(h.subs[0].1.as_deref(), Some("%4"));
}

#[test]
fn test_on_rejects_an_unknown_option_key() {
    let rt = rt();
    let mut h = FakeHost::default();
    let e = eval::<()>(
        &rt,
        &mut h,
        "grofe.on('pane-closed', function() end, {pane = '%1', scope = 'x'})",
    )
    .unwrap_err();
    assert!(e.to_string().contains("unknown option scope"), "{e}");
    assert!(h.subs.is_empty());
}

#[test]
fn test_context_passes_host_fields_through() {
    let rt = rt();
    let mut h = FakeHost::default();
    let out: String = eval(
        &rt,
        &mut h,
        "local c = grofe.context() return tostring(c.client) .. tostring(c.pane) .. tostring(c.cwd)",
    )
    .unwrap();
    assert_eq!(out, "client-1nil/w");
}

#[test]
fn test_context_fields_are_nil_when_the_host_has_none() {
    let rt = rt();
    let mut h = FakeHost {
        context: Some(Context::default()),
        ..FakeHost::default()
    };
    let out: String = eval(
        &rt,
        &mut h,
        "local c = grofe.context() return tostring(c.client) .. tostring(c.pane) .. tostring(c.cwd)",
    )
    .unwrap();
    assert_eq!(out, "nilnilnil");
}

#[test]
fn test_choose_sends_labels_and_previews_and_maps_the_pick_to_the_value() {
    let rt = rt();
    let mut h = FakeHost::default();
    eval::<()>(
        &rt,
        &mut h,
        "grofe.choose({'a', {label = 'B', value = 42}}, function(v) got = v end, \
         {title = 'T', preview = function(i) return type(i) end})",
    )
    .unwrap();
    let (job, id) = h.chooses[0].clone();
    assert_eq!(job.labels, ["a", "B"]);
    assert_eq!(job.previews, ["string", "table"]);
    assert_eq!(job.title.as_deref(), Some("T"));
    call(&rt, &mut h, id, Payload::Pick(Some(2))).unwrap();
    let got: f64 = eval(&rt, &mut h, "return got").unwrap();
    assert_eq!(got, 42.0);
    eval::<()>(&rt, &mut h, "grofe.choose({'a'}, function(v) got = v end)").unwrap();
    let second = h.chooses[1].1;
    call(&rt, &mut h, second, Payload::Pick(None)).unwrap();
    assert!(
        eval::<Option<f64>>(&rt, &mut h, "return got")
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_choose_with_no_items_answers_nil_at_once() {
    let rt = rt();
    let mut h = FakeHost::default();
    let out: String = eval(
        &rt,
        &mut h,
        "local r = 'unset' grofe.choose({}, function(v) r = tostring(v) end) return r",
    )
    .unwrap();
    assert_eq!(out, "nil");
    assert!(h.chooses.is_empty());
}

#[test]
fn test_choose_and_input_reject_bad_arguments() {
    let rt = rt();
    let mut h = FakeHost::default();
    for src in [
        "grofe.choose('x', function() end)",
        "grofe.choose({'a'}, 1)",
        "grofe.choose({'a', 5}, function() end)",
        "grofe.choose({{value = 1}}, function() end)",
        "grofe.choose({'a'}, function() end, {preview = 1})",
        "grofe.input(5, function() end)",
        "grofe.input('p', 'x')",
    ] {
        let e = eval::<()>(&rt, &mut h, src).expect_err(src).to_string();
        assert!(
            e.contains("grofe.choose") || e.contains("grofe.input"),
            "{src}: {e}"
        );
    }
    assert!(h.chooses.is_empty() && h.inputs.is_empty());
}

#[test]
fn test_input_reaches_host_and_text_reaches_the_callback() {
    let rt = rt();
    let mut h = FakeHost::default();
    eval::<()>(
        &rt,
        &mut h,
        "grofe.input('Name: ', function(v) got = v end)",
    )
    .unwrap();
    let (prompt, id) = h.inputs[0].clone();
    assert_eq!(prompt, "Name: ");
    call(&rt, &mut h, id, Payload::Text(Some("hi".into()))).unwrap();
    let got: String = eval(&rt, &mut h, "return got").unwrap();
    assert_eq!(got, "hi");
}

#[test]
fn test_notify_reply_option_reaches_host() {
    let rt = rt();
    let mut h = FakeHost::default();
    eval::<()>(
        &rt,
        &mut h,
        "grofe.notify('a', {reply = true}) grofe.notify('b')",
    )
    .unwrap();
    assert_eq!(h.notified, [true, false]);
}
