//! Timers, event subscriptions and child processes for Lua callbacks. Each
//! runs off the actor and posts a `Job` that calls `lua_host::call_job`.

use crate::server::spawn::{Command, Stdio};
use std::collections::HashMap;
use std::os::unix::process::ExitStatusExt;
use std::time::Duration;

use nix::sys::signal::Signal;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinHandle};

use crate::lua::api::{Payload, SystemJob};
use crate::server::agents::headless;
use crate::server::events::{QUEUE, Scope};
use crate::server::{Job, State, events::Events, lua_host};

#[derive(Default)]
pub(crate) struct Async {
    tasks: HashMap<u32, AbortHandle>,
    /// The generation of the runtime this belongs to (see `lua_host`).
    generation: u64,
}

/// A job for runtime `generation`, run only while that runtime is the one
/// running: callback ids restart at each reload, so what an earlier runtime
/// queued must not reach the new one. Every job that names a callback is made
/// here.
pub(crate) fn guarded(generation: u64, f: impl FnOnce(&mut State) + Send + 'static) -> Job {
    Box::new(move |state| {
        if lua_host::is_current(state, generation) {
            f(state);
        }
    })
}

fn post(generation: u64, id: u32, payload: Payload) -> Job {
    guarded(generation, move |state| {
        lua_host::call_job(state, id, payload)
    })
}

/// The job that ends a `system` run: the job's list entry ends (and the tray
/// hears of it), then the callback runs.
fn finished(generation: u64, id: u32, payload: Payload, cause: Option<String>) -> Job {
    guarded(generation, move |state| {
        if let Payload::System { code, .. } = &payload {
            headless::finish(state, generation, id, *code, cause);
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
    lines: Option<(mpsc::Sender<Job>, u64, u32)>,
) -> JoinHandle<String> {
    tokio::spawn(async move {
        let mut all = Vec::new();
        if let Some(mut p) = pipe {
            let mut chunk = [0u8; 8192];
            // Where the line being read starts in `all`, and how far `all`
            // has been searched for its end.
            let mut start = 0;
            let mut scanned = 0;
            loop {
                let n = match p.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                all.extend_from_slice(&chunk[..n]);
                while let Some((jobs, generation, id)) = &lines
                    && let Some(nl) = all[scanned..].iter().position(|b| *b == b'\n')
                {
                    let end = scanned + nl;
                    let line = line_text(&all[start..end]);
                    start = end + 1;
                    scanned = start;
                    if jobs
                        .send(post(*generation, *id, Payload::Line(line)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                scanned = all.len();
            }
            if let Some((jobs, generation, id)) = &lines
                && start < all.len()
            {
                let line = line_text(&all[start..]);
                let _ = jobs.send(post(*generation, *id, Payload::Line(line))).await;
            }
        }
        String::from_utf8_lossy(&all).into_owned()
    })
}

/// A line's text, without one trailing `\r` (CRLF output).
fn line_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Kills the child's process group when dropped (a cancelled job), unless
/// the job ended on its own, and stops the tasks that serve the job.
struct Group {
    pgid: Option<i32>,
    tasks: Vec<AbortHandle>,
}

impl Group {
    fn kill(&mut self) {
        if let Some(pgid) = self.pgid.take() {
            let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pgid), Signal::SIGKILL);
        }
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.kill();
        for t in &self.tasks {
            t.abort();
        }
    }
}

/// How long the pipes get to close after a timeout killed the group.
// ponytail: a descendant that left the group (setsid) and holds a pipe keeps
// it open; after this the output is given up on rather than awaited forever.
const PIPE_GRACE: Duration = Duration::from_secs(2);

/// Runs `job`: what the callback gets, and the cause when the job did not
/// run to its own exit (a failed spawn, a timeout), for its tray entry.
async fn run_system(
    job: SystemJob,
    generation: u64,
    jobs: mpsc::Sender<Job>,
) -> (Payload, Option<String>) {
    let failed = |msg: String| {
        let payload = Payload::System {
            code: -1,
            stdout: String::new(),
            stderr: msg.clone(),
            timed_out: false,
            error: Some(msg.clone()),
            signal: None,
        };
        (payload, Some(msg))
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
    let mut child = match crate::server::spawn::spawn(&mut cmd) {
        Ok(c) => c,
        Err(e) => return failed(format!("{prog}: {e}")),
    };
    let deadline = tokio::time::Instant::now() + Duration::from_millis(job.timeout_ms);
    let mut group = Group {
        pgid: child.id().and_then(|p| i32::try_from(p).ok()),
        tasks: Vec::new(),
    };
    if let (Some(text), Some(mut pipe)) = (job.stdin, child.stdin.take()) {
        // Off this task: input bigger than the pipe waits for the child to read.
        let writer = tokio::spawn(async move {
            let _ = pipe.write_all(text.as_bytes()).await;
        });
        group.tasks.push(writer.abort_handle());
    }
    let mut out_task = read_lines(
        child.stdout.take(),
        job.on_line.map(|id| (jobs, generation, id)),
    );
    let mut err_task = read_all(child.stderr.take());
    group.tasks.push(out_task.abort_handle());
    group.tasks.push(err_task.abort_handle());
    // One deadline over the exit and both pipes: a descendant that holds a
    // pipe open outlives the child, and is as much the job as the child.
    let mut code = None;
    let mut signal = None;
    let ran = tokio::time::timeout_at(deadline, async {
        code = Some(match child.wait().await {
            Ok(status) => {
                signal = status.signal();
                status.code().unwrap_or(-1)
            }
            Err(_) => -1,
        });
        let out = (&mut out_task).await.unwrap_or_default();
        (out, (&mut err_task).await.unwrap_or_default())
    })
    .await;
    let mut cause = None;
    let mut timed_out = false;
    let (code, stdout, stderr) = match ran {
        Ok((out, err)) => {
            if let Some(n) = signal {
                cause = Some(format!("killed by signal {n}"));
            }
            (code.unwrap_or(-1), out, err)
        }
        Err(_) => {
            group.kill();
            let _ = child.wait().await;
            let pipes = async {
                let out = (&mut out_task).await.unwrap_or_default();
                (out, (&mut err_task).await.unwrap_or_default())
            };
            let (out, err) = tokio::time::timeout(PIPE_GRACE, pipes)
                .await
                .unwrap_or_default();
            timed_out = true;
            signal = None;
            cause = Some(format!("timed out after {}ms", job.timeout_ms));
            (-1, out, err)
        }
    };
    // Finished on its own: what it left running is its own business.
    group.pgid = None;
    let payload = Payload::System {
        code,
        stdout,
        stderr,
        timed_out,
        error: None,
        signal,
    };
    (payload, cause)
}

impl Async {
    pub(crate) fn new(generation: u64) -> Async {
        Async {
            generation,
            ..Async::default()
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Forgets the task of `id` without stopping it; whether there was one.
    pub(crate) fn forget(&mut self, id: u32) -> bool {
        self.tasks.remove(&id).is_some()
    }

    fn track(&mut self, id: u32, handle: AbortHandle) {
        if let Some(old) = self.tasks.insert(id, handle) {
            old.abort();
        }
    }

    pub(crate) fn every(&mut self, jobs: &mpsc::Sender<Job>, ms: u64, id: u32) {
        let jobs = jobs.clone();
        let generation = self.generation;
        let period = Duration::from_millis(ms);
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if jobs
                    .send(post(generation, id, Payload::None))
                    .await
                    .is_err()
                {
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
        events.subscribe_owned(vec![name.to_string()], scope, tx)?;
        let jobs = jobs.clone();
        let generation = self.generation;
        let task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let value = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);
                if jobs
                    .send(post(generation, id, Payload::Event(value)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            // The server dropped the subscription: its scope is gone.
            let end = guarded(generation, move |state| {
                lua_host::forget_subscription(state, id)
            });
            let _ = jobs.send(end).await;
        });
        self.track(id, task.abort_handle());
        Ok(())
    }

    /// Runs `job`; when it ends, its callback `id` runs.
    pub(crate) fn system(&mut self, jobs: &mpsc::Sender<Job>, job: SystemJob, id: u32) {
        let jobs = jobs.clone();
        let generation = self.generation;
        let task = tokio::spawn(async move {
            let (payload, cause) = run_system(job, generation, jobs.clone()).await;
            let _ = jobs.send(finished(generation, id, payload, cause)).await;
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
        run_system(job, 0, tx).await.0
    }

    #[tokio::test]
    async fn test_lines_split_across_chunks_are_joined() {
        let (tx, mut rx) = mpsc::channel::<Job>(16);
        let (mut w, r) = tokio::io::duplex(64);
        let task = read_lines(Some(r), Some((tx, 0, 1)));
        for part in ["ab", "c\nd", "e\n\nf"] {
            w.write_all(part.as_bytes()).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(w);
        assert_eq!(task.await.unwrap(), "abc\nde\n\nf");
        let mut state = crate::server::test_state();
        let mut seen = 0;
        while let Ok(job) = rx.try_recv() {
            job(&mut state);
            seen += 1;
        }
        assert_eq!(seen, 4);
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
                ..
            } => assert_eq!((code, stdout.as_str(), stderr.as_str()), (3, "hi\n", "e\n")),
            _ => panic!("not a system payload"),
        }
        let (tx, mut rx) = mpsc::channel::<Job>(1);
        Async::default().system(&tx, job(vec!["true".into()], Duration::from_secs(5)), 7);
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
