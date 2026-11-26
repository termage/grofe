//! The `agent` command's shapes.

use super::{AgentReport, Target};

/// `agent report|list|next|seen` (grofe's own): native agent status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentCmd {
    Report(AgentReport),
    List { format: Option<String> },
    Next,
    Seen { target: Target, all: bool },
    New(AgentNew),
}

/// `agent new <kind> [-pPdwW] [-c dir] [-n name] [-b branch] [-e K=V]
/// [-- command...]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentNew {
    pub kind: String,
    pub split: bool,
    pub cwd: Option<String>,
    pub name: Option<String>,
    /// `-w` is `Some(true)`, `-W` is `Some(false)`, neither is `None`.
    pub worktree: Option<bool>,
    pub branch: Option<String>,
    pub detached: bool,
    /// `-P`: print the new pane's id.
    pub print: bool,
    /// `-e`: variables for the new pane's environment.
    pub env: Vec<(String, String)>,
    /// The words after `--`: run instead of the kind's command.
    pub command: Vec<String>,
}
