//! Git worktree grouping (spec 2026-11-26-issue-agents, A3): the sidebar and
//! the picker list the sessions whose cwd is a linked worktree under one row
//! named after the main repo's directory, each labelled with its branch.
//!
//! The repo is found as `git rev-parse --git-common-dir` finds it, from the
//! `.git` file and `commondir` of the worktree, read as files: no git process
//! runs on the actor.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use crate::server::State;
use crate::types::PaneId;

/// Where a session's cwd sits in a git repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Info {
    /// The repo's git common dir: what its worktrees share.
    pub common: PathBuf,
    /// The main repo's directory name.
    pub repo: String,
    /// The branch checked out now (a short commit when detached).
    pub branch: String,
    /// A linked worktree, not the main checkout.
    pub linked: bool,
}

/// What resolving a cwd found, apart from the branch, which moves.
#[derive(Debug, Clone)]
struct Repo {
    common: PathBuf,
    repo: String,
    linked: bool,
    head: PathBuf,
}

// ponytail: the repo of a cwd is looked up once and kept for the server's
// life (only the branch is read again, so a checkout shows at once); a
// directory that becomes a repo, or a repo that moves, is seen after a
// restart. It is a cache so a render reads no more than one small file per
// worktree session and never runs git.
static CACHE: LazyLock<Mutex<HashMap<PathBuf, Option<Repo>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The `.git` directory a checkout at `dir` uses, and whether it is a
/// linked worktree's own.
fn git_dir(dir: &Path) -> Option<(PathBuf, bool)> {
    let dotgit = dir.join(".git");
    if dotgit.is_dir() {
        return Some((dotgit, false));
    }
    let text = fs::read_to_string(&dotgit).ok()?;
    let path = text.strip_prefix("gitdir:")?.trim();
    let gitdir = dir.join(path);
    let linked = gitdir.join("commondir").is_file();
    Some((gitdir, linked))
}

fn resolve(cwd: &Path) -> Option<Repo> {
    let (dir, (gitdir, linked)) = cwd.ancestors().find_map(|d| git_dir(d).map(|g| (d, g)))?;
    let common = match fs::read_to_string(gitdir.join("commondir")) {
        Ok(rel) => gitdir.join(rel.trim()),
        Err(_) => gitdir.clone(),
    };
    let common = common.canonicalize().ok()?;
    let named = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    let repo = match named(&common).as_deref() {
        Some(".git") => common.parent().and_then(named),
        Some(n) => Some(n.strip_suffix(".git").unwrap_or(n).to_string()),
        None => named(dir),
    }?;
    Some(Repo {
        common,
        repo,
        linked,
        head: gitdir.join("HEAD"),
    })
}

fn branch_of(head: &Path) -> Option<String> {
    let text = fs::read_to_string(head).ok()?;
    let text = text.trim();
    Some(match text.strip_prefix("ref: ") {
        Some(r) => r.strip_prefix("refs/heads/").unwrap_or(r).to_string(),
        None => text.chars().take(7).collect(),
    })
}

/// Where `cwd` sits in a git repository, if it does.
pub(crate) fn of_cwd(cwd: &Path) -> Option<Info> {
    let mut cache = CACHE.lock().ok()?;
    let repo = cache
        .entry(cwd.to_path_buf())
        .or_insert_with(|| resolve(cwd));
    let mut branch = branch_of(&repo.as_ref()?.head);
    // A worktree that was removed or moved: look again.
    if branch.is_none() {
        *repo = resolve(cwd);
        branch = branch_of(&repo.as_ref()?.head);
    }
    let r = repo.as_ref()?;
    Some(Info {
        common: r.common.clone(),
        repo: r.repo.clone(),
        branch: branch?,
        linked: r.linked,
    })
}

/// Where `pane`'s session sits in a git repository, if it does.
pub(crate) fn of_pane(state: &State, pane: PaneId) -> Option<Info> {
    let model = &state.model;
    let window = model.panes.get(&pane)?.window;
    let session = model.windows.get(&window)?.session;
    of_cwd(&model.sessions.get(&session)?.cwd)
}

/// One entry of a list in display order: an item on its own, or a repo and
/// the items under it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Block {
    One(usize),
    Repo { repo: String, members: Vec<usize> },
}

/// `infos` (one per item) as blocks: the items of a repo that has a linked
/// worktree among them gather under one block, where its first item was; the
/// rest stay where they are.
pub(crate) fn arrange(infos: &[Option<Info>]) -> Vec<Block> {
    let grouped: HashSet<&PathBuf> = infos
        .iter()
        .flatten()
        .filter(|i| i.linked)
        .map(|i| &i.common)
        .collect();
    let mut done: HashSet<&PathBuf> = HashSet::new();
    let mut blocks = Vec::new();
    for (n, info) in infos.iter().enumerate() {
        let Some(i) = info.as_ref().filter(|i| grouped.contains(&i.common)) else {
            blocks.push(Block::One(n));
            continue;
        };
        if done.insert(&i.common) {
            let members = (0..infos.len())
                .filter(|m| infos[*m].as_ref().is_some_and(|o| o.common == i.common))
                .collect();
            blocks.push(Block::Repo {
                repo: i.repo.clone(),
                members,
            });
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// `repo/` with one linked worktree `wt/` on branch `feat/x`.
    fn layout() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().canonicalize().unwrap();
        let (main, wt) = (base.join("repo"), base.join("wt"));
        write(&main.join(".git/HEAD"), "ref: refs/heads/main\n");
        let admin = main.join(".git/worktrees/wt");
        write(&admin.join("HEAD"), "ref: refs/heads/feat/x\n");
        write(&admin.join("commondir"), "../..\n");
        write(&wt.join(".git"), &format!("gitdir: {}\n", admin.display()));
        (root, main, wt)
    }

    #[test]
    fn a_worktree_resolves_to_its_main_repo_and_branch() {
        let (_root, main, wt) = layout();
        let sub = wt.join("src/deep");
        fs::create_dir_all(&sub).unwrap();
        let w = of_cwd(&sub).unwrap();
        assert_eq!(
            (w.repo.as_str(), w.branch.as_str(), w.linked),
            ("repo", "feat/x", true)
        );
        let m = of_cwd(&main).unwrap();
        assert_eq!((m.branch.as_str(), m.linked), ("main", false));
        assert_eq!(w.common, m.common);
    }

    #[test]
    fn the_branch_is_read_again_and_a_detached_head_is_a_short_commit() {
        let (_root, _main, wt) = layout();
        assert_eq!(of_cwd(&wt).unwrap().branch, "feat/x");
        let head = wt.join(".git");
        let admin = fs::read_to_string(&head).unwrap();
        let admin = Path::new(admin.trim().strip_prefix("gitdir: ").unwrap());
        fs::write(admin.join("HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(of_cwd(&wt).unwrap().branch, "0123456");
    }

    #[test]
    fn a_dir_outside_git_has_no_info() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(of_cwd(root.path()), None);
    }

    fn info(common: &str, repo: &str, linked: bool) -> Option<Info> {
        Some(Info {
            common: PathBuf::from(common),
            repo: repo.into(),
            branch: "b".into(),
            linked,
        })
    }

    #[test]
    fn linked_worktrees_gather_where_the_first_of_them_is() {
        let infos = [
            info("/a", "a", true),
            None,
            info("/b", "b", false),
            info("/a", "a", true),
            info("/a", "a", false),
        ];
        assert_eq!(
            arrange(&infos),
            vec![
                Block::Repo {
                    repo: "a".into(),
                    members: vec![0, 3, 4]
                },
                Block::One(1),
                Block::One(2),
            ]
        );
    }

    #[test]
    fn a_main_checkout_alone_is_no_group() {
        let infos = [info("/a", "a", false), info("/a", "a", false)];
        assert_eq!(arrange(&infos), vec![Block::One(0), Block::One(1)]);
    }
}
