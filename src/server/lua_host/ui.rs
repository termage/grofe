//! `grofe.choose` and `grofe.input`: the client they open on, and how a
//! prompt's answer finds its way back to the callback.

use crate::lua::api::Payload;
use crate::protocol::ClientCtx;
use crate::server::input::Routed;
use crate::server::{Job, State, clients, lua_async, lua_host, prompt};
use crate::types::ClientId;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::mpsc;

/// The prompts `grofe.input` has open, shared with their [`Reply`]s, which
/// leave it when their prompt closes.
#[derive(Default)]
pub(crate) struct Inputs(Arc<Mutex<Open>>);

#[derive(Default)]
struct Open {
    clients: BTreeSet<ClientId>,
    /// Callbacks whose answer found the job queue full, freed at the next
    /// input, the next drained job or a reload.
    stranded: Vec<(u64, u32)>,
}

fn lock(open: &Mutex<Open>) -> MutexGuard<'_, Open> {
    open.lock().unwrap_or_else(|e| e.into_inner())
}

/// A reload ends the runtime the open input prompts' callbacks live in, so
/// they close unanswered.
pub(crate) fn clear(state: &mut State) {
    let clients = {
        let mut open = lock(&state.pickers.inputs.0);
        open.stranded.clear();
        std::mem::take(&mut open.clients)
    };
    for client in clients {
        prompt::client_gone(state, client);
        clients::mark_dirty(state, client);
    }
}

/// The client a chooser or prompt opens on: the key press's, else the one a
/// command would act on, with the context its callback runs in (the key
/// press's, else the client's own).
pub(super) fn client_of(
    state: &State,
    press: Option<&ClientCtx>,
    ctx: &ClientCtx,
) -> Result<(ClientId, ClientCtx), String> {
    let client =
        clients::current_client(state, press.unwrap_or(ctx)).ok_or("no client to show it on")?;
    let own = state.clients.get(&client).map(|c| c.ctx.clone());
    let press = press.cloned().or(own).ok_or("no client to show it on")?;
    Ok((client, press))
}

/// A prompt's answer on its way to a Lua callback: sent once, as the text
/// the prompt gave or, when it is dropped unanswered (cancelled or replaced),
/// as nothing.
struct Reply {
    generation: u64,
    id: u32,
    press: ClientCtx,
    jobs: mpsc::Sender<Job>,
    client: ClientId,
    open: Arc<Mutex<Open>>,
    sent: bool,
}

impl Reply {
    fn send(&mut self, text: Option<String>) {
        if std::mem::replace(&mut self.sent, true) {
            return;
        }
        let (id, press) = (self.id, self.press.clone());
        let job = lua_async::guarded(self.generation, move |state| {
            lua_host::call_once_for(state, &press, id, Payload::Text(text));
        });
        // A full job queue drops the answer; the callback is freed at the
        // next input, as a chooser frees its own.
        if self.jobs.try_send(job).is_err() {
            lock(&self.open).stranded.push((self.generation, self.id));
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        self.send(None);
        lock(&self.open).clients.remove(&self.client);
    }
}

/// Frees the callbacks whose answer found the job queue full.
pub(crate) fn release_stranded(state: &State) {
    let stranded = std::mem::take(&mut lock(&state.pickers.inputs.0).stranded);
    for (generation, id) in stranded {
        lua_host::release_once(state, generation, id);
    }
}

/// Opens a one-line prompt on `client` whose answer goes to callback `id`
/// of runtime `generation`.
pub(super) fn input(
    state: &mut State,
    client: ClientId,
    press: ClientCtx,
    (generation, id): (u64, u32),
    label: &str,
) {
    let open = state.pickers.inputs.0.clone();
    release_stranded(state);
    let mut reply = Reply {
        generation,
        id,
        press,
        jobs: state.jobs.clone(),
        client,
        open: open.clone(),
        sent: false,
    };
    let then = Box::new(move |_: &mut State, _: ClientId, line: &str| {
        reply.send(Some(line.to_string()));
        Routed::Consumed
    });
    prompt::ask(state, client, label, "", "search", false, then);
    // After the ask: a prompt it replaced leaves the set as it drops.
    lock(&open).clients.insert(client);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_ctx;

    #[test]
    fn an_answer_that_finds_the_queue_full_strands_its_callback_and_a_drop_leaves_the_set() {
        let (jobs, _rx) = mpsc::channel::<Job>(1);
        jobs.try_send(Box::new(|_: &mut State| {})).unwrap();
        let open = Arc::new(Mutex::new(Open::default()));
        lock(&open).clients.insert(ClientId(4));
        let mut reply = Reply {
            generation: 7,
            id: 3,
            press: test_ctx(),
            jobs,
            client: ClientId(4),
            open: open.clone(),
            sent: false,
        };
        reply.send(Some("x".into()));
        assert_eq!(lock(&open).stranded, [(7, 3)]);
        drop(reply);
        assert!(lock(&open).clients.is_empty());
    }

    /// A state with callbacks 1 and 2 registered; its queue is full, and
    /// callback 1 has been stranded by a dropped Reply.
    fn stranded_state() -> (State, mpsc::Receiver<Job>) {
        let (mut state, rx) = crate::server::test_state_at("/nonexistent/ui.sock".into());
        let code = "grofe.every(3600000, function() end) grofe.every(3600000, function() end)";
        lua_host::run_chunk(&mut state, &test_ctx(), code).unwrap();
        while state.jobs.try_send(Box::new(|_: &mut State| {})).is_ok() {}
        let generation = state.lua.as_ref().unwrap().async_.generation();
        let reply = Reply {
            generation,
            id: 1,
            press: test_ctx(),
            jobs: state.jobs.clone(),
            client: ClientId(4),
            open: state.pickers.inputs.0.clone(),
            sent: false,
        };
        drop(reply);
        (state, rx)
    }

    fn freed(state: &State, id: u32) -> bool {
        let lua = state.lua.as_ref().unwrap();
        let callbacks: mlua::Table = lua
            .rt
            .lua()
            .named_registry_value("grofe_callbacks")
            .unwrap();
        matches!(callbacks.get::<mlua::Value>(id), Ok(mlua::Value::Nil))
    }

    #[tokio::test]
    async fn a_stranded_callback_is_freed_at_the_next_input() {
        let (mut state, _rx) = stranded_state();
        assert!(!freed(&state, 1));
        let generation = state.lua.as_ref().unwrap().async_.generation();
        input(&mut state, ClientId(5), test_ctx(), (generation, 2), "q");
        assert!(freed(&state, 1));
        assert!(!freed(&state, 2));
    }

    #[tokio::test]
    async fn a_stranded_callback_is_freed_when_a_job_drains_without_another_input() {
        let (mut state, mut rx) = stranded_state();
        let job = rx.recv().await.unwrap();
        crate::server::run_job(&mut state, job);
        assert!(freed(&state, 1));
    }

    #[tokio::test]
    async fn a_reload_forgets_the_stranded_callbacks() {
        let (mut state, _rx) = stranded_state();
        clear(&mut state);
        assert!(lock(&state.pickers.inputs.0).stranded.is_empty());
    }
}
