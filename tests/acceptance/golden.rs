//! The tmux copy-mode goldens' scenario format, shared by the TM suites:
//! a golden's lines split into the commands, clients and keys it records.

use std::path::PathBuf;

pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/compat")
}

/// A scenario line's words, split as the replayer splits them.
pub fn split_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            ' ' | '\t' if word.is_some() => words.extend(word.take()),
            ' ' | '\t' => {}
            '\'' => {
                let w = word.get_or_insert_default();
                for c in it.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    w.push(c);
                }
            }
            '"' => {
                let w = word.get_or_insert_default();
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' => match it.next() {
                            Some(n @ ('"' | '\\' | '$' | '`')) => w.push(n),
                            Some(n) => {
                                w.push('\\');
                                w.push(n);
                            }
                            None => w.push('\\'),
                        },
                        c => w.push(c),
                    }
                }
            }
            '\\' => word.get_or_insert_default().extend(it.next()),
            c => word.get_or_insert_default().push(c),
        }
    }
    words.extend(word);
    words
}

/// The bytes a `keys` step types (the recorder's `key_bytes`).
pub fn key_bytes(words: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for word in words {
        match word.as_str() {
            "Space" => out.push(b' '),
            "Escape" => out.push(0x1b),
            "Enter" => out.push(b'\r'),
            "Tab" => out.push(b'\t'),
            "BSpace" => out.push(0x7f),
            w => match w.strip_prefix("C-").map(str::as_bytes) {
                Some([c]) => out.push(c.to_ascii_lowercase() - b'a' + 1),
                _ => out.extend_from_slice(w.as_bytes()),
            },
        }
    }
    out
}

pub enum Step {
    Run {
        line: String,
        args: Vec<String>,
        want: (String, String, i32),
    },
    Client(u16, u16),
    Keys(Vec<u8>),
}

pub fn load(name: &str, golden: &str) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    for line in golden.lines() {
        if let Some(args) = line.strip_prefix("$ ") {
            let args = split_words(args)
                .into_iter()
                .map(|a| {
                    a.replace("{fixtures}", &fixtures().display().to_string())
                        .replace("{grofe}", env!("CARGO_BIN_EXE_grofe"))
                })
                .collect();
            steps.push(Step::Run {
                line: line.to_string(),
                args,
                want: (String::new(), String::new(), 0),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix("client ") {
            let size = rest.split(' ').next().unwrap();
            let (c, r) = size.split_once('x').unwrap();
            steps.push(Step::Client(c.parse().unwrap(), r.parse().unwrap()));
            continue;
        }
        if let Some(rest) = line.strip_prefix("keys ") {
            steps.push(Step::Keys(key_bytes(&split_words(rest))));
            continue;
        }
        let Some(Step::Run { want, .. }) = steps.last_mut() else {
            panic!("{name}: output before a command: {line}");
        };
        if let Some(status) = line.strip_prefix("? ") {
            want.2 = status.parse().unwrap();
        } else if let Some(err) = line.strip_prefix("! ") {
            want.1.push_str(err);
            want.1.push('\n');
        } else {
            want.0.push_str(line);
            want.0.push('\n');
        }
    }
    steps
}
