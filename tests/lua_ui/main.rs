//! Spec 2026-11-26-issue-agents, Part A, item A3: `grofe.choose`,
//! `grofe.input`, the callback context, `agent-sidebar-format` and
//! `agent-grid-format`, tray reply, and worktree grouping. Each test has a
//! private socket and temp dirs; the UI is driven by keys typed into an
//! attached client's pty.

mod choose;
#[path = "../common/mod.rs"]
mod common;
mod context;
mod formats;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../acceptance/harness.rs"]
mod harness;
mod helpers;
mod input;
#[allow(dead_code)]
#[path = "../lua_runtime_api/support.rs"]
mod support;
mod tray_reply;
mod worktree;
