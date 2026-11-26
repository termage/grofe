//! `grofe.choose` and `grofe.input`: the client they open on, and how a
//! prompt's answer finds its way back to the callback.

use crate::lua::api::Payload;
use crate::protocol::ClientCtx;
use crate::server::input::Routed;
use crate::server::{Job, State, clients, lua_async, lua_host, prompt};
use crate::types::ClientId;
use tokio::sync::mpsc;

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
        // ponytail: a full job queue drops the answer; its callback stays
        // registered until the next reload.
        let _ = self.jobs.try_send(job);
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        self.send(None);
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
    let mut reply = Reply {
        generation,
        id,
        press,
        jobs: state.jobs.clone(),
        sent: false,
    };
    let then = Box::new(move |_: &mut State, _: ClientId, line: &str| {
        reply.send(Some(line.to_string()));
        Routed::Consumed
    });
    prompt::ask(state, client, label, "", "search", false, then);
}
