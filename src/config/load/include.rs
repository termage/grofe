//! `include` and its alias `source-file` (spec §5.1, A10): other files
//! loaded at the including file's layer, after its own keys.

use std::fs;
use std::path::Path;

use toml::de::DeValue;

use super::{FileCtx, Loader, canonical, tilde};

impl Loader {
    /// `key = "x.toml"` or `key = ["a.toml", "b.toml"]`, where `key` is
    /// `include` or `source-file`.
    pub(super) fn include_list(
        &mut self,
        f: &FileCtx,
        line: usize,
        key: &str,
        v: &DeValue,
        out: &mut Vec<(String, usize)>,
    ) {
        let bad = |found: &str| format!("expected a path or an array of paths, found {found}");
        match v {
            DeValue::String(s) => out.push((s.to_string(), line)),
            DeValue::Array(a) => {
                for item in a {
                    let item_line = f.line(item.span().start);
                    match item.get_ref() {
                        DeValue::String(s) => out.push((s.to_string(), item_line)),
                        other => {
                            let msg = bad(other.type_str());
                            self.error(f.path, item_line, key, msg);
                        }
                    }
                }
            }
            other => self.error(f.path, line, key, bad(other.type_str())),
        }
    }

    /// Loads `rel`, resolved against the including file's directory (spec
    /// §5.1), at the including file's layer. Errors name `key`.
    pub(super) fn include(&mut self, f: &FileCtx, line: usize, key: &str, rel: &str) {
        let dir = f.path.parent().unwrap_or(Path::new(""));
        let path = dir.join(rel);
        let canon = canonical(&path);
        if self.stack.contains(&canon) {
            let chain: Vec<String> = self
                .stack
                .iter()
                .chain([&canon])
                .map(|p| tilde(p, self.home.as_deref()))
                .collect();
            let msg = format!("include cycle: {}", chain.join(" -> "));
            return self.error(f.path, line, key, msg);
        }
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                let msg = format!("cannot read {}: {e}", path.display());
                return self.error(f.path, line, key, msg);
            }
        };
        self.stack.push(canon);
        self.load_text(&path, &text, f.layer);
        self.stack.pop();
    }
}
