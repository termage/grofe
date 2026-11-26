//! Spec 2026-11-26-issue-agents, Part A, item A2a: `grofe.system` options,
//! `agent new`, headless agent records, agent messages and history, and
//! pane lifecycle events. Each test has a private socket and temp dirs.

#[path = "../common/mod.rs"]
mod common;
mod events;
mod headless;
mod helpers;
mod history;
mod new;
#[allow(dead_code)]
#[path = "../lua_runtime_api/support.rs"]
mod support;
mod system;
