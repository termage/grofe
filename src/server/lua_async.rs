//! Timers, event subscriptions and child processes for Lua callbacks. Each
//! runs off the actor and posts a `Job` that calls `lua_host::call_job`.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use nix::sys::signal::Signal;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinHandle};

use crate::lua::api::{Payload, SystemJob};
use crate::server::agents::headless;
use crate::server::events::{QUEUE, Scope};
use crate::server::{Job, events::Events, lua_host};
use crate::types::PaneId;

#[derive(Default)]
pub(crate) struct Async {
    tasks: HashMap<u32, AbortHandle>,
}

fn post(id: u32, payload: Payload) -> Job {
    Box::new(move |state| lua_host::call_job(state, id, payload))
}

/// The job that ends a `system` run: its headless record is finished and
/// forgotten, then the callback runs.
fn finished(id: u32, payload: Payload, headless: Option<PaneId>) -> Job {
    Box::new(move |state| {
        if let Some(pane) = headless {
            if let Payload::System { code, .. } = &payload {
                headless::finish(state, pane, *code);
            }
            lua_host::forget_headless(state, id);
        }
        lua_host::call_job(state, id, payload)
    })
}

fn read_all<R: AsyncRead + Unpin + Send + 'static>(pipe: Option<R>) -> JoinHandle<String> {
    read_lines(pipe, None)
}

/// Reads `pipe` to its end; each line goes to `lines`' callback as it
/// arrives, the last one even without a newline.
fn read_lines<R: AsyncRead + Unpin + Send + 'static>(
    pipe: Option<R>,
    lines: Option<(mpsc::Sender<Job>, u32)>,
) -> JoinHandle<String> {
    tokio::spawn(async move {
        let mut all = Vec::new();
        if let Some(mut p) = pipe {
            let mut chunk = [0u8; 8192];
            // Where the line being read starts in `all`.
            let mut start = 0;
            loop {
                let n = match p.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                all.extend_from_slice(&chunk[..n]);
                while let Some((jobs, id)) = &lines
                    && let Some(nl) = all[start..].iter().position(|b| *b == b'\n')
                {
                    let line = String::from_utf8_lossy(&all[start..start + nl]).into_owned();
                    start += nl + 1;
                    if jobs.send(post(*id, Payload::Line(line))).await.is_err() {
                        break;
                    }
                }
            }
            if let Some((jobs, id)) = &lines
                && start < all.len()
            {
                let line = String::from_utf8_lossy(&all[start..]).into_owned();
                let _ = jobs.send(post(*id, Payload::Line(line))).await;
            }
        }
        String::from_utf8_lossy(&all).into_owned()
    })
}

/// Kills the child's process group when dropped (a cancelled job), unless
/// the job ended on its own.
struct Group(Option<i32>);

impl Group {
    fn kill(&mut self) {
        if let Some(pgid) = self.0.take() {
            let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pgid), Signal::SIGKILL);
        }
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.kill();
    }
}

async fn run_system(job: SystemJob, jobs: mpsc::Sender<Job>) -> Payload {
    let failed = |msg: String| Payload::System {
        code: -1,
        stdout: String::new(),
        stderr: msg,
    };
    let Some((prog, args)) = job.argv.split_first() else {
        return failed("empty argv".into());
    };
    let mut cmd = Command::new(prog);
    cmd.args(args)
        .envs(job.env.iter().map(|(k, v)| (k, v)))
        .stdin(if job.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    if let Some(cwd) = &job.cwd {
        cmd.current_dir(cwd);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return failed(format!("{prog}: {e}")),
    };
    let mut group = Group(child.id().and_then(|p| i32::try_from(p).ok()));
    if let (Some(text), Some(mut pipe)) = (job.stdin, child.stdin.take()) {
        // Off this task: input bigger than the pipe waits for the child to read.
        tokio::spawn(async move {
            let _ = pipe.write_all(text.as_bytes()).await;
        });
    }
    let out_task = read_lines(child.stdout.take(), job.on_line.map(|id| (jobs, id)));
    let err_task = read_all(child.stderr.take());
    let timeout = Duration::from_millis(job.timeout_ms);
    let code = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status.code().unwrap_or(-1),
        Ok(Err(_)) => -1,
        Err(_) => {
            group.kill();
            let _ = child.wait().await;
            -1
        }
    };
    // It ended by itself: what it left running is its own business.
    group.0 = None;
    let stdout = out_task.await.unwrap_or_default();
    let stderr = err_task.await.unwrap_or_default();
    Payload::System {
        code,
        stdout,
        stderr,
    }
}

impl Async {
    fn track(&mut self, id: u32, handle: AbortHandle) {
        if let Some(old) = self.tasks.insert(id, handle) {
            old.abort();
        }
    }

    pub(crate) fn every(&mut self, jobs: &mpsc::Sender<Job>, ms: u64, id: u32) {
        let jobs = jobs.clone();
        let period = Duration::from_millis(ms);
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if jobs.send(post(id, Payload::None)).await.is_err() {
                    break;
                }
            }
        });
        self.track(id, task.abort_handle());
    }

    pub(crate) fn subscribe(
        &mut self,
        events: &mut Events,
        jobs: &mpsc::Sender<Job>,
        name: &str,
        scope: Option<Scope>,
        id: u32,
    ) -> Result<(), String> {
        let (tx, mut rx) = mpsc::channel(QUEUE);
        events.subscribe(vec![name.to_string()], scope, tx)?;
        let jobs = jobs.clone();
        let task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let value = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);
                if jobs.send(post(id, Payload::Event(value))).await.is_err() {
                    break;
                }
            }
        });
        self.track(id, task.abort_handle());
        Ok(())
    }

    /// Runs `job`; `headless` is the agent record that follows it, finished
    /// when the job ends, just before its callback runs.
    pub(crate) fn system(
        &mut self,
        jobs: &mpsc::Sender<Job>,
        job: SystemJob,
        headless: Option<PaneId>,
        id: u32,
    ) {
        let jobs = jobs.clone();
        let task = tokio::spawn(async move {
            let payload = run_system(job, jobs.clone()).await;
            let _ = jobs.send(finished(id, payload, headless)).await;
        });
        self.track(id, task.abort_handle());
    }

    pub(crate) fn cancel(&mut self, id: u32) {
        if let Some(handle) = self.tasks.remove(&id) {
            handle.abort();
        }
    }

    pub(crate) fn cancel_all(&mut self) {
        for (_, handle) in self.tasks.drain() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(argv: Vec<String>, timeout: Duration) -> SystemJob {
        SystemJob {
            argv,
            cwd: None,
            timeout_ms: timeout.as_millis() as u64,
            env: Vec::new(),
            stdin: None,
            on_line: None,
            agent: None,
        }
    }

    async fn run(job: SystemJob) -> Payload {
        let (tx, _rx) = mpsc::channel::<Job>(16);
        run_system(job, tx).await
    }

    #[tokio::test]
    async fn test_cancel_stops_timer() {
        let (tx, mut rx) = mpsc::channel::<Job>(16);
        let mut a = Async::default();
        a.every(&tx, 20, 1);
        assert!(rx.recv().await.is_some());
        a.cancel(1);
        while rx.try_recv().is_ok() {}
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_every_posts_repeatedly_and_cancel_all_stops() {
        let (tx, mut rx) = mpsc::channel::<Job>(16);
        let mut a = Async::default();
        a.every(&tx, 20, 1);
        a.every(&tx, 20, 2);
        for _ in 0..3 {
            assert!(rx.recv().await.is_some());
        }
        a.cancel_all();
        while rx.try_recv().is_ok() {}
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_system_timeout_kills() {
        let dir = std::env::temp_dir().join(format!("grofe-t4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("pid");
        let script = format!("echo $$ > {}; exec sleep 5", pidfile.display());
        let t = std::time::Instant::now();
        let p = run(job(
            vec!["sh".into(), "-c".into(), script],
            Duration::from_millis(200),
        ))
        .await;
        assert!(t.elapsed() < Duration::from_secs(3));
        assert!(matches!(p, Payload::System { code: -1, .. }));
        let pid = std::fs::read_to_string(&pidfile).unwrap();
        let alive = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(!alive, "child {pid} still running");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_system_captures_output_and_posts_a_job() {
        let p = run(job(
            vec![
                "sh".into(),
                "-c".into(),
                "echo hi; echo e >&2; exit 3".into(),
            ],
            Duration::from_secs(5),
        ))
        .await;
        match p {
            Payload::System {
                code,
                stdout,
                stderr,
            } => assert_eq!((code, stdout.as_str(), stderr.as_str()), (3, "hi\n", "e\n")),
            _ => panic!("not a system payload"),
        }
        let (tx, mut rx) = mpsc::channel::<Job>(1);
        Async::default().system(
            &tx,
            job(vec!["true".into()], Duration::from_secs(5)),
            None,
            7,
        );
        assert!(rx.recv().await.is_some());
    }

    #[tokio::test]
    async fn test_unknown_event_errors() {
        let (tx, _rx) = mpsc::channel::<Job>(1);
        let mut events = Events::default();
        let e = Async::default()
            .subscribe(&mut events, &tx, "no-such-event", None, 1)
            .unwrap_err();
        assert!(e.contains("no-such-event"), "{e}");
        Async::default()
            .subscribe(&mut events, &tx, "pane-exited", None, 2)
            .unwrap();
    }
}
