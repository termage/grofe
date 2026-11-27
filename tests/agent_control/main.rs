//! A2b of the bifurc spec (docs/specs/2026-11-26-issue-agents.md): agent
//! prompt and read, detection files, explain, unseen done, the sidebar
//! roll-up and resume after a restore. Everything goes through the CLI and
//! Lua on private sockets, against a fake agent script.

#[path = "../common/mod.rs"]
mod common;
mod detect;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../acceptance/harness.rs"]
mod harness;
mod helpers;
mod prompt;
mod read;
mod resume;
mod rollup;
mod unseen;
