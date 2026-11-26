use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Luau,
    Tmux,
}

#[derive(Deserialize)]
struct Manifest {
    name: String,
    /// The oldest grofe plugin API the pack runs on.
    min_api: Option<u32>,
    /// The earlier spelling of `min_api`.
    api: Option<u32>,
    #[serde(default = "empty_options")]
    options: toml::Value,
}

fn empty_options() -> toml::Value {
    toml::Value::Table(toml::Table::new())
}

impl Manifest {
    fn check_api(&self) -> Result<(), String> {
        let need = self
            .min_api
            .or(self.api)
            .ok_or_else(|| format!("{}: the manifest needs min_api", self.name))?;
        if need > super::PLUGIN_API {
            return Err(format!(
                "{} needs grofe plugin API {need}, this grofe has {}",
                self.name,
                super::PLUGIN_API
            ));
        }
        Ok(())
    }
}

fn read_manifest(dir: &Path) -> Result<Option<Manifest>, String> {
    let path = dir.join("grofe-plugin.toml");
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text)
        .map(Some)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The pack's `[options]` defaults as JSON; `{}` without a manifest.
pub(crate) fn default_options(dir: &Path) -> Result<serde_json::Value, String> {
    match read_manifest(dir)? {
        Some(m) if !m.options.is_table() => Err(format!("{}: options must be a table", m.name)),
        Some(m) => crate::lua::json::toml_to_json(&m.options)
            .map_err(|e| format!("{}: options: {e}", m.name)),
        None => Ok(serde_json::json!({})),
    }
}

/// Classify a plugin directory and refuse a Luau pack that needs a newer
/// API than this grofe has.
pub(crate) fn detect(dir: &Path) -> Result<Kind, String> {
    if let Some(m) = read_manifest(dir)? {
        m.check_api()?;
    }
    kind(dir)
}

/// Classify a plugin directory: `grofe-plugin.toml` wins over any `*.tmux`
/// file. A pack with `min_api` is not checked, so `pack sync` can fetch one
/// that only a newer grofe can load; the older `api` still fails here.
pub(crate) fn kind(dir: &Path) -> Result<Kind, String> {
    if let Some(m) = read_manifest(dir)? {
        if m.min_api.is_none() {
            m.check_api()?;
        }
        if !dir.join("init.luau").is_file() && !dir.join("init.lua").is_file() {
            return Err(format!("{}: no init.luau", m.name));
        }
        return Ok(Kind::Luau);
    }
    let has_tmux = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .any(|e| e.path().extension().is_some_and(|x| x == "tmux") && e.path().is_file());
    if has_tmux {
        Ok(Kind::Tmux)
    } else {
        Err("not a grofe or tmux plugin".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (n, c) in files {
            std::fs::write(d.path().join(n), c).unwrap();
        }
        d
    }

    #[test]
    fn test_manifest_wins_over_tmux_file() {
        let d = dir(&[
            ("grofe-plugin.toml", "name = \"p\"\napi = 1\n"),
            ("init.luau", "return {}"),
            ("yank.tmux", ""),
        ]);
        assert_eq!(detect(d.path()), Ok(Kind::Luau));
    }

    #[test]
    fn test_bad_manifest_toml_names_the_file() {
        let d = dir(&[("grofe-plugin.toml", "name = ")]);
        let e = detect(d.path()).unwrap_err();
        assert!(e.contains("grofe-plugin.toml"), "{e}");
    }

    #[test]
    fn test_api_mismatch() {
        let d = dir(&[("grofe-plugin.toml", "name = \"p\"\napi = 2\n")]);
        assert_eq!(
            detect(d.path()),
            Err("p needs grofe plugin API 2, this grofe has 1".into())
        );
    }

    #[test]
    fn test_min_api_loads_at_or_below_and_refuses_above() {
        let ok = dir(&[
            ("grofe-plugin.toml", "name = \"p\"\nmin_api = 1\n"),
            ("init.luau", ""),
        ]);
        assert_eq!(detect(ok.path()), Ok(Kind::Luau));
        let new = dir(&[
            ("grofe-plugin.toml", "name = \"p\"\nmin_api = 9\n"),
            ("init.luau", ""),
        ]);
        assert_eq!(
            detect(new.path()),
            Err("p needs grofe plugin API 9, this grofe has 1".into())
        );
        assert_eq!(kind(new.path()), Ok(Kind::Luau));
    }

    #[test]
    fn test_manifest_without_an_api_is_refused() {
        let d = dir(&[("grofe-plugin.toml", "name = \"p\"\n"), ("init.luau", "")]);
        assert!(detect(d.path()).unwrap_err().contains("min_api"));
    }

    #[test]
    fn test_default_options_are_json() {
        let d = dir(&[(
            "grofe-plugin.toml",
            "name = \"p\"\nmin_api = 1\n[options]\na = 1\n[options.t]\nb = \"x\"\n",
        )]);
        assert_eq!(
            default_options(d.path()),
            Ok(serde_json::json!({"a": 1, "t": {"b": "x"}}))
        );
    }

    #[test]
    fn test_tmux_file_is_tmux() {
        let d = dir(&[("yank.tmux", "")]);
        assert_eq!(detect(d.path()), Ok(Kind::Tmux));
    }

    #[test]
    fn test_empty_dir_is_not_a_plugin() {
        let d = dir(&[]);
        assert_eq!(detect(d.path()), Err("not a grofe or tmux plugin".into()));
    }

    #[test]
    fn test_luau_needs_init() {
        let d = dir(&[("grofe-plugin.toml", "name = \"p\"\napi = 1\n")]);
        assert_eq!(detect(d.path()), Err("p: no init.luau".into()));
        std::fs::write(d.path().join("init.lua"), "").unwrap();
        assert_eq!(detect(d.path()), Ok(Kind::Luau));
    }
}
