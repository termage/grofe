//! `agent new <kind> [-pPdwW] [-c dir] [-n name] [-b branch] [-e K=V]
//! [-- command...]`.

use super::{ParsedFlags, env_values, flags_for, perr, too_many};
use crate::cmd::{AgentCmd, AgentNew, Command, ParseError};

/// Reads what follows `agent new`.
pub(super) fn parse(rest: &[String]) -> Result<Command, ParseError> {
    // The kind may come first (`agent new claude -w`), so flags are read
    // from what follows it.
    let (lead, flag_argv) = match rest.split_first() {
        Some((k, tail)) if !k.starts_with('-') => (Some(k.clone()), tail),
        _ => (None, rest),
    };
    // Words after `--` are the command, in place of the kind's.
    let (flag_argv, command) = match flag_argv.iter().position(|a| a == "--") {
        Some(at) if at + 1 == flag_argv.len() => {
            return Err(perr("agent new: -- needs a command"));
        }
        Some(at) => (&flag_argv[..at], flag_argv[at + 1..].to_vec()),
        None => (flag_argv, Vec::new()),
    };
    let flags: ParsedFlags = flags_for(flag_argv, "pc:n:wWb:dPe:", "agent new")?;
    if flags
        .repeated
        .iter()
        .any(|(c, v)| *c == 'e' && !v.contains('='))
    {
        return Err(perr("agent new: -e takes K=V"));
    }
    let mut positionals = flags.positionals.clone();
    if let Some(k) = lead {
        positionals.insert(0, k);
    }
    if positionals.len() > 1 {
        return Err(too_many("agent new", 1));
    }
    let kind = positionals
        .pop()
        .ok_or_else(|| perr("agent new: kind required"))?;
    let worktree = match (flags.on('w'), flags.on('W')) {
        (true, true) => return Err(perr("agent new: -w and -W conflict")),
        (true, false) => Some(true),
        (false, true) => Some(false),
        (false, false) => None,
    };
    Ok(Command::Agent(AgentCmd::New(AgentNew {
        kind,
        split: flags.on('p'),
        cwd: flags.val('c'),
        name: flags.val('n'),
        worktree,
        branch: flags.val('b'),
        detached: flags.on('d'),
        print: flags.on('P'),
        env: env_values(&flags),
        command,
    })))
}

#[cfg(test)]
mod tests {
    use crate::cmd::{AgentCmd, AgentNew, Command, parse};

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn agent_new_example() {
        assert_eq!(
            parse(&argv(&["agent", "new", "claude", "-w", "-b", "feat/x"])),
            Ok(Command::Agent(AgentCmd::New(AgentNew {
                kind: "claude".into(),
                split: false,
                cwd: None,
                name: None,
                worktree: Some(true),
                branch: Some("feat/x".into()),
                detached: false,
                print: false,
                env: Vec::new(),
                command: Vec::new(),
            })))
        );
        let Ok(Command::Agent(AgentCmd::New(n))) = parse(&argv(&[
            "agent", "new", "pi", "-p", "-d", "-W", "-c", "/r", "-n", "nm",
        ])) else {
            panic!("parse")
        };
        assert!(n.split && n.detached);
        assert_eq!(n.worktree, Some(false));
        assert_eq!(n.cwd.as_deref(), Some("/r"));
        assert_eq!(n.name.as_deref(), Some("nm"));
        assert!(parse(&argv(&["agent", "new"])).is_err());
    }

    #[test]
    fn w_and_big_w_conflict() {
        assert!(parse(&argv(&["agent", "new", "claude", "-w", "-W"])).is_err());
    }

    #[test]
    fn words_after_double_dash_are_the_command() {
        let Ok(Command::Agent(AgentCmd::New(n))) = parse(&argv(&[
            "agent", "new", "claude", "-d", "--", "sh", "-c", "x",
        ])) else {
            panic!("parse")
        };
        assert_eq!(n.command, ["sh", "-c", "x"]);
        assert!(n.detached);
    }

    #[test]
    fn double_dash_with_nothing_after_is_an_error() {
        let err = parse(&argv(&["agent", "new", "claude", "--"])).expect_err("no command");
        assert!(err.to_string().contains("needs a command"), "{err}");
        assert!(parse(&argv(&["agent", "new", "claude", "-d", "--"])).is_err());
    }
}
