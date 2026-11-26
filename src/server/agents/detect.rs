//! Screen detection files (`<config dir>/agents/<kind>.toml`): regexes for
//! `working`, `blocked` and `idle` matched on the last `lines` of a pane's
//! screen. Files for the common kinds are built in; a user file replaces the
//! built-in one for its kind.

use std::collections::HashMap;
use std::path::Path;

use regex::{Regex, RegexBuilder};
use serde::Deserialize;

use super::AgentState;

const DEFAULT_LINES: usize = 20;

/// The built-in files, by kind.
const BUILTIN: [(&str, &str); 5] = [
    ("claude", include_str!("detect/claude.toml")),
    ("codex", include_str!("detect/codex.toml")),
    ("gemini", include_str!("detect/gemini.toml")),
    ("opencode", include_str!("detect/opencode.toml")),
    ("aider", include_str!("detect/aider.toml")),
];

/// Where a record's current state came from, for `agent explain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    Report,
    Osc,
    Bell,
    Screen { file: String, rule: String },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Raw {
    working: Option<OneOrMany>,
    blocked: Option<OneOrMany>,
    idle: Option<OneOrMany>,
    lines: Option<usize>,
}

/// One kind's compiled detection file.
#[derive(Debug, Clone)]
pub(crate) struct DetectFile {
    /// Where it came from: a path, or `built-in <kind>`.
    pub source: String,
    lines: usize,
    /// In the order they are checked.
    rules: Vec<(AgentState, Vec<Regex>)>,
}

impl DetectFile {
    /// Compiles `text`; the error names `source` and the key at fault.
    pub(crate) fn parse(source: &str, text: &str) -> Result<DetectFile, String> {
        let raw: Raw =
            toml::from_str(text).map_err(|e| format!("{source}: {}", e.message().trim()))?;
        let mut rules = Vec::new();
        for (key, state, value) in [
            ("working", AgentState::Working, raw.working),
            ("blocked", AgentState::Blocked, raw.blocked),
            ("idle", AgentState::Idle, raw.idle),
        ] {
            let patterns = match value {
                None => Vec::new(),
                Some(OneOrMany::One(p)) => vec![p],
                Some(OneOrMany::Many(ps)) => ps,
            };
            let compiled = patterns
                .iter()
                .map(|p| {
                    RegexBuilder::new(p)
                        .multi_line(true)
                        .build()
                        .map_err(|e| format!("{source}: {key}: invalid regex {p:?}: {e}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            rules.push((state, compiled));
        }
        Ok(DetectFile {
            source: source.to_string(),
            lines: raw.lines.unwrap_or(DEFAULT_LINES),
            rules,
        })
    }

    /// The state the last `lines` lines of `screen` show, and the rule that
    /// matched, `working` first. Blank lines at the bottom do not count.
    pub(crate) fn detect(&self, screen: &str) -> Option<(AgentState, String)> {
        let lines: Vec<&str> = screen.trim_end().lines().collect();
        let tail = lines[lines.len().saturating_sub(self.lines)..].join("\n");
        self.rules.iter().find_map(|(state, res)| {
            let hit = res.iter().find(|re| re.is_match(&tail))?;
            let key = state.as_str();
            Some((*state, format!("{key} = {}", hit.as_str())))
        })
    }
}

/// Every kind's file: the built-in ones, each replaced by `<config
/// dir>/agents/<kind>.toml` when that exists; a user file that does not
/// compile is reported, and the built-in one stays. A user file for a kind
/// with no built-in one is loaded too.
pub(crate) fn load(config_dir: Option<&Path>) -> (HashMap<String, DetectFile>, Vec<String>) {
    let mut files = HashMap::new();
    let mut errors = Vec::new();
    for (kind, text) in BUILTIN {
        match DetectFile::parse(&format!("built-in {kind}"), text) {
            Ok(f) => {
                files.insert(kind.to_string(), f);
            }
            Err(e) => errors.push(e),
        }
    }
    let Some(dir) = config_dir.map(|d| d.join("agents")) else {
        return (files, errors);
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (files, errors);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = match (path.extension(), path.file_stem().and_then(|s| s.to_str())) {
            (Some(ext), Some(stem)) if ext == "toml" => stem.to_string(),
            _ => continue,
        };
        let shown = path.display().to_string();
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| format!("{shown}: {e}"))
            .and_then(|text| DetectFile::parse(&shown, &text));
        match parsed {
            Ok(f) => {
                files.insert(kind, f);
            }
            Err(e) => errors.push(e),
        }
    }
    (files, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_files_load_and_a_user_file_replaces_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (files, errors) = load(Some(dir.path()));
        assert!(errors.is_empty(), "{errors:?}");
        for kind in ["claude", "codex", "gemini", "opencode", "aider"] {
            assert!(files[kind].source.starts_with("built-in"), "{kind}");
        }
        let hit = files["claude"].detect("Do you want to proceed?\n1. Yes");
        assert_eq!(hit.map(|(s, _)| s), Some(AgentState::Blocked));
        assert_eq!(files["claude"].detect("hello\n"), None);

        std::fs::create_dir(dir.path().join("agents")).expect("agents dir");
        std::fs::write(dir.path().join("agents/claude.toml"), "idle = 'HELLO'").expect("write");
        let (files, errors) = load(Some(dir.path()));
        assert!(errors.is_empty(), "{errors:?}");
        assert!(files["claude"].source.ends_with("agents/claude.toml"));
        assert_eq!(
            files["claude"].detect("hello HELLO"),
            Some((AgentState::Idle, "idle = HELLO".into()))
        );
        assert_eq!(files["claude"].detect("Do you want to proceed?"), None);
    }

    #[test]
    fn an_invalid_regex_names_file_and_key_and_keeps_the_built_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("agents")).expect("agents dir");
        std::fs::write(
            dir.path().join("agents/claude.toml"),
            "working = 'ok'\nblocked = ['fine', 'bad(']\n",
        )
        .expect("write");
        let (files, errors) = load(Some(dir.path()));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("claude.toml") && errors[0].contains("blocked"));
        assert!(files["claude"].source.starts_with("built-in"));
    }

    #[test]
    fn working_is_checked_first_and_only_the_last_lines_count() {
        let f = DetectFile::parse(
            "t",
            "lines = 2\nworking = 'WORK'\nblocked = 'ASK'\nidle = ['^>$']\n",
        )
        .expect("parse");
        let state = |s: &str| f.detect(s).map(|(s, _)| s);
        assert_eq!(state("WORK\nASK\n\n\n"), Some(AgentState::Working));
        assert_eq!(state("ASK\nx\ny"), None, "ASK is above the last 2 lines");
        assert_eq!(state("x\n> "), Some(AgentState::Idle));
    }
}
