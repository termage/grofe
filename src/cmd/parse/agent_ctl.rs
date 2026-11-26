//! `agent prompt|read|explain`, and the error for any other word.

use super::perr;
use crate::cmd::target::parse_target;
use crate::cmd::{AgentCmd, AgentPrompt, Command, ParseError, Target};

/// Reads `agent <sub> <rest...>` for the subcommands that are not `report`,
/// `list`, `next`, `seen` or `new`.
pub(super) fn parse(sub: &str, rest: &[String]) -> Result<Command, ParseError> {
    let cmd = match sub {
        "prompt" => AgentCmd::Prompt(prompt(rest)?),
        "read" => read(rest)?,
        "explain" => AgentCmd::Explain {
            target: pane_only("agent explain", rest)?,
        },
        other => return Err(perr(format!("agent: unknown subcommand {other}"))),
    };
    Ok(Command::Agent(cmd))
}

fn pane_only(name: &str, rest: &[String]) -> Result<Target, ParseError> {
    match rest {
        [pane] => parse_target(pane),
        [] => Err(perr(format!("{name}: pane required"))),
        _ => Err(perr(format!("{name}: too many arguments"))),
    }
}

fn read(rest: &[String]) -> Result<AgentCmd, ParseError> {
    let (mut pane, mut lines) = (None, None);
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "-n" {
            let v = rest
                .get(i + 1)
                .ok_or_else(|| perr("agent read: -n needs a value"))?;
            lines = Some(
                v.parse::<usize>()
                    .map_err(|_| perr(format!("agent read: bad line count {v}")))?,
            );
            i += 2;
        } else if rest[i].starts_with('-') && rest[i] != "-" {
            return Err(perr(format!("agent read: unknown flag {}", rest[i])));
        } else {
            if pane.replace(rest[i].clone()).is_some() {
                return Err(perr("agent read: too many arguments"));
            }
            i += 1;
        }
    }
    let pane = pane.ok_or_else(|| perr("agent read: pane required"))?;
    Ok(AgentCmd::Read {
        target: parse_target(&pane)?,
        lines,
    })
}

/// `<pane> [--wait] [--until a,b] [--timeout s] -- <text...>`.
fn prompt(rest: &[String]) -> Result<AgentPrompt, ParseError> {
    const NAME: &str = "agent prompt";
    let at = rest
        .iter()
        .position(|a| a == "--")
        .filter(|at| at + 1 < rest.len())
        .ok_or_else(|| perr(format!("{NAME}: the text goes after --")))?;
    let (flags, text) = (&rest[..at], rest[at + 1..].join(" "));
    let (mut pane, mut wait, mut until, mut timeout) = (None, false, Vec::new(), None);
    let mut i = 0;
    while i < flags.len() {
        let tok = flags[i].as_str();
        let mut value = || {
            i += 1;
            flags
                .get(i)
                .ok_or_else(|| perr(format!("{NAME}: {tok} needs a value")))
        };
        match tok {
            "--wait" => wait = true,
            "--until" => {
                until = value()?.split(',').map(str::to_string).collect();
            }
            "--timeout" => {
                let v = value()?;
                timeout = Some(
                    v.parse::<u64>()
                        .map_err(|_| perr(format!("{NAME}: bad --timeout {v}")))?,
                );
            }
            t if t.starts_with('-') && t != "-" => {
                return Err(perr(format!("{NAME}: unknown flag {t}")));
            }
            t => {
                if pane.replace(t.to_string()).is_some() {
                    return Err(perr(format!("{NAME}: too many arguments")));
                }
            }
        }
        i += 1;
    }
    let pane = pane.ok_or_else(|| perr(format!("{NAME}: pane required")))?;
    if !wait && (!until.is_empty() || timeout.is_some()) {
        return Err(perr(format!("{NAME}: --until and --timeout need --wait")));
    }
    Ok(AgentPrompt {
        target: parse_target(&pane)?,
        text,
        wait,
        until,
        timeout_secs: timeout,
    })
}

#[cfg(test)]
mod tests {
    use crate::cmd::{AgentCmd, Command, parse};

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn prompt_reads_flags_and_the_text_after_the_dashes() {
        let Ok(Command::Agent(AgentCmd::Prompt(p))) = parse(&argv(&[
            "agent",
            "prompt",
            "%3",
            "--wait",
            "--until",
            "idle,blocked",
            "--timeout",
            "9",
            "--",
            "fix",
            "the bug",
        ])) else {
            panic!("a prompt");
        };
        assert_eq!(p.text, "fix the bug");
        assert!(p.wait);
        assert_eq!(p.until, ["idle", "blocked"]);
        assert_eq!(p.timeout_secs, Some(9));
    }

    #[test]
    fn prompt_needs_text_and_a_pane() {
        assert!(parse(&argv(&["agent", "prompt", "%3"])).is_err());
        assert!(parse(&argv(&["agent", "prompt", "%3", "--"])).is_err());
        assert!(parse(&argv(&["agent", "prompt", "--", "x"])).is_err());
        assert!(
            parse(&argv(&[
                "agent", "prompt", "%3", "--until", "idle", "--", "x"
            ]))
            .is_err()
        );
    }

    #[test]
    fn read_and_explain_take_a_pane() {
        assert!(matches!(
            parse(&argv(&["agent", "read", "%3", "-n", "5"])),
            Ok(Command::Agent(AgentCmd::Read { lines: Some(5), .. }))
        ));
        assert!(matches!(
            parse(&argv(&["agent", "read", "%3"])),
            Ok(Command::Agent(AgentCmd::Read { lines: None, .. }))
        ));
        assert!(parse(&argv(&["agent", "read"])).is_err());
        assert!(matches!(
            parse(&argv(&["agent", "explain", "%3"])),
            Ok(Command::Agent(AgentCmd::Explain { .. }))
        ));
        assert!(parse(&argv(&["agent", "explain"])).is_err());
    }
}
