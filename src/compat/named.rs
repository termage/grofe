//! The parses of [`super`], each command with the canonical name of the
//! verb it came from (`#{command}`, tmux's `cmdq_merge_formats`): the row
//! name of grofe's table, or tmux's command name, aliases resolved. A tmux
//! command the front end translates into several native ones names each.

use super::{logged, tmux};
use crate::cmd::{self, Command, ParseError};
use crate::protocol::{ClientCtx, Dialect};

/// A command and the verb it was parsed from.
pub type Named = (&'static str, Command);

/// The canonical name of the verb `name`, in `ctx`'s dialect.
fn verb(ctx: &ClientCtx, name: &str) -> &'static str {
    let found = match ctx.dialect {
        Dialect::Grofe => cmd::verb_name(name),
        Dialect::Tmux => tmux::resolve(name).ok().map(|s| s.name),
    };
    found.unwrap_or_default()
}

fn named(ctx: &ClientCtx, argv: &[String], cmds: Vec<Command>) -> Vec<Named> {
    let name = verb(ctx, argv.first().map_or("", String::as_str));
    cmds.into_iter().map(|c| (name, c)).collect()
}

/// [`super::parse_request`], named: the request split at `;` arguments,
/// each command translated before any runs, a failure logged once for the
/// whole request.
pub fn request(ctx: &ClientCtx, argv: &[String]) -> Result<Vec<Named>, String> {
    let mut out = Vec::new();
    for words in cmd::parse::split_argv(argv) {
        let cmds = match ctx.dialect {
            Dialect::Grofe => cmd::parse(&words)
                .map(|c| vec![c])
                .map_err(|e| e.to_string())?,
            Dialect::Tmux => logged(argv, tmux::dispatch_command(&words, ctx))?,
        };
        out.extend(named(ctx, &words, cmds));
    }
    if out.is_empty() && ctx.dialect == Dialect::Grofe {
        return Err(ParseError::EmptyCommand.to_string());
    }
    Ok(out)
}

/// [`super::parse_split`], named.
pub fn split(ctx: &ClientCtx, argv: &[String]) -> Result<Vec<Named>, String> {
    Ok(named(ctx, argv, super::parse_split(ctx, argv)?))
}

/// [`super::parse_split_checked`], named.
pub fn split_checked(ctx: &ClientCtx, argv: &[String]) -> Result<Vec<Named>, (bool, String)> {
    Ok(named(ctx, argv, super::parse_split_checked(ctx, argv)?))
}
