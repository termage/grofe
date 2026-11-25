//! `include` and `source-file` (spec 2026-11-25-config-dir-and-shrink, A10, t9).

use super::super::*;

fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in files {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

fn load_main(dir: &tempfile::TempDir) -> Result<Resolved, Vec<ConfigError>> {
    load_one(&dir.path().join("main.toml"), &Env::from_vars(None, None))
}

fn left(dir: &tempfile::TempDir) -> Option<String> {
    load_main(dir)
        .expect("loads")
        .get("status.left")
        .map(str::to_string)
}

/// t9 (A10): `include` takes one path.
#[test]
fn include_takes_a_string() {
    let dir = tree(&[
        ("main.toml", "include = \"a.toml\"\n"),
        ("a.toml", "status.left = \"A\"\n"),
    ]);
    assert_eq!(left(&dir).as_deref(), Some("A"));
}

/// t9 (A10): `include` takes a list, applied in order.
#[test]
fn include_takes_a_list_applied_in_order() {
    let dir = tree(&[
        ("main.toml", "include = [\"a.toml\", \"b.toml\"]\n"),
        ("a.toml", "status.left = \"A\"\nstatus.right = \"RA\"\n"),
        ("b.toml", "status.left = \"B\"\n"),
    ]);
    let r = load_main(&dir).expect("loads");
    assert_eq!(r.get("status.left"), Some("B"));
    assert_eq!(r.get("status.right"), Some("RA"));
}

/// t9 (A10): paths are relative to the including file, and the
/// included files apply after its own keys.
#[test]
fn include_is_relative_to_the_including_file_and_applies_after_its_keys() {
    let dir = tree(&[
        (
            "main.toml",
            "status.left = \"M\"\ninclude = \"sub/a.toml\"\n",
        ),
        ("sub/a.toml", "status.left = \"A\"\ninclude = \"b.toml\"\n"),
        ("sub/b.toml", "status.right = \"RB\"\n"),
    ]);
    let r = load_main(&dir).expect("loads");
    assert_eq!(r.get("status.left"), Some("A"));
    assert_eq!(r.get("status.right"), Some("RB"));
}

/// t9 (A10): `include = []` is fine and includes nothing.
#[test]
fn an_empty_include_list_is_fine() {
    let dir = tree(&[("main.toml", "status.left = \"M\"\ninclude = []\n")]);
    assert_eq!(left(&dir).as_deref(), Some("M"));
}

/// t9 (A10): both keys, `include` first and then `source-file`.
#[test]
fn include_applies_before_source_file() {
    let dir = tree(&[
        (
            "main.toml",
            "source-file = \"s.toml\"\ninclude = \"i.toml\"\n",
        ),
        ("i.toml", "status.left = \"I\"\n"),
        ("s.toml", "status.left = \"S\"\n"),
    ]);
    assert_eq!(left(&dir).as_deref(), Some("S"));
}

/// t9 (A10): `source-file` is still an alias.
#[test]
fn source_file_still_works() {
    let dir = tree(&[
        ("main.toml", "source-file = \"s.toml\"\n"),
        ("s.toml", "status.left = \"S\"\n"),
    ]);
    assert_eq!(left(&dir).as_deref(), Some("S"));
}

/// t9 (A10): an include cycle, a file including itself, is an error that
/// names `include`.
#[test]
fn an_include_cycle_names_the_include_key() {
    let dir = tree(&[("main.toml", "include = \"main.toml\"\n")]);
    let errors = load_main(&dir).expect_err("cycle");
    assert!(
        errors
            .iter()
            .any(|e| e.key == "include" && e.message.contains("cycle")),
        "{errors:?}"
    );
}

/// t9 (A10): a missing file is an error that names `include`.
#[test]
fn a_missing_include_names_the_include_key() {
    let dir = tree(&[("main.toml", "include = \"nope.toml\"\n")]);
    let errors = load_main(&dir).expect_err("missing");
    assert!(
        errors
            .iter()
            .any(|e| e.key == "include" && e.message.contains("nope.toml")),
        "{errors:?}"
    );
}

/// t9 (A10): a wrong type is an error that names `include`.
#[test]
fn include_of_a_number_is_a_type_error_naming_the_key() {
    let dir = tree(&[("main.toml", "include = 3\n")]);
    let errors = load_main(&dir).expect_err("type error");
    assert!(
        errors
            .iter()
            .any(|e| e.key == "include" && e.message.contains("expected a path")),
        "{errors:?}"
    );
}

/// t9 (A10): errors name the key that was actually used.
#[test]
fn source_file_errors_still_name_source_file() {
    let dir = tree(&[("main.toml", "source-file = 3\n")]);
    let errors = load_main(&dir).expect_err("type error");
    assert!(errors.iter().any(|e| e.key == "source-file"), "{errors:?}");
    let dir = tree(&[("main.toml", "source-file = \"nope.toml\"\n")]);
    let errors = load_main(&dir).expect_err("missing");
    assert!(errors.iter().any(|e| e.key == "source-file"), "{errors:?}");
}
