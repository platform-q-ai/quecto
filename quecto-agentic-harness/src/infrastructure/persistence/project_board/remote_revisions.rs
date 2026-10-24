//! Which revisions a repository's remote holds (#2666), with git, as the
//! [`RemoteRevisions`] port: what a run's end judges a success by, and
//! what a teardown asks of a board run's completion. Its own adapter, apart
//! from the board store (#2705 B1), so a board kept anywhere else still
//! judges by the repository's remote.
//!
//! A revision is held when it names a commit (its full id, or a prefix of
//! at least 12 hex digits) that one of the remote's branches or tags
//! reaches. The remote's tips are listed (`ls-remote`), each commit is
//! walked first against the tips this clone holds, and only when they do
//! not reach it are the tips it lacks fetched by id (no ref, tag or
//! `FETCH_HEAD` moves) and it is walked again. A commit this clone does not
//! hold once every tip's history is here is reached by none. Text that
//! names no commit is never held, and asks nothing of the remote. A shallow
//! clone's walk stops at its boundary, so a commit past it reads as not
//! held: a success is then refused, never recorded wrongly.
use super::board_tree::{object_id, unavailable};
use super::git::{Git, SHORT_OUTPUT, failed};
use super::store::{GitBoardStore, StoreOpenError, configured_remote};
use crate::application::project_board::ports::{BoardStoreError, RemoteRevisions};
use std::collections::BTreeSet;

/// The most a listing of the remote's branches and tags takes.
const MAX_TIP_LISTING: usize = 16 * 1024 * 1024;

/// The shortest prefix of a commit id taken as naming one, as the port
/// reads one (the push rule's own bound).
const MIN_PREFIX: usize = 12;

/// `revision` as the commit id, or prefix of one, it spells, in lower
/// case: 12 to 64 hex digits (an allowlist), and nothing else.
fn commit_named(revision: &str) -> Option<String> {
    let hex = revision.bytes().all(|b| b.is_ascii_hexdigit());
    let long_enough = (MIN_PREFIX..=64).contains(&revision.len());
    (hex && long_enough).then(|| revision.to_ascii_lowercase())
}

/// The revisions one configured remote of a repository holds, read with
/// git.
pub struct GitRemoteRevisions {
    git: Git,
    remote: String,
}

impl GitRemoteRevisions {
    /// `remote` of the repository `git` runs in: a remote the repository
    /// configures, checked as the board store checks its own, so a path or
    /// URL spelled like a name is never taken for one.
    ///
    /// # Errors
    /// A name that is not a configured remote's; git that cannot say.
    pub fn new(git: Git, remote: &str) -> Result<Self, StoreOpenError> {
        configured_remote(&git, remote)?;
        Ok(Self {
            git,
            remote: remote.to_owned(),
        })
    }

    /// The remote `store` is opened on, read with the store's own git: no
    /// git is asked, since the store's remote was checked (or vouched for)
    /// as it was opened.
    pub fn of_store(store: &GitBoardStore) -> Self {
        Self {
            git: store.git().clone(),
            remote: store.remote().to_owned(),
        }
    }
}

impl RemoteRevisions for GitRemoteRevisions {
    fn not_on_remote(&self, revisions: &[String]) -> Result<Vec<String>, BoardStoreError> {
        let named: Vec<(usize, String)> = revisions
            .iter()
            .enumerate()
            .filter_map(|(at, revision)| commit_named(revision).map(|commit| (at, commit)))
            .collect();
        let mut held = BTreeSet::new();
        // Only a revision naming a commit asks the remote anything.
        match named.as_slice() {
            [] => {}
            [_, ..] => {
                let tips = self.remote_tips()?;
                // The tips this clone holds first: a fetch only for what
                // they do not reach (round 1, #2670).
                let local = self.tip_commits(&tips)?;
                for (at, commit) in &named {
                    if self.reached(commit, &local)? {
                        held.insert(*at);
                    }
                }
                let (_, unreached): (Vec<_>, Vec<_>) =
                    named.iter().partition(|(at, _)| held.contains(at));
                let lacking = self.lacking(&tips)?;
                if let ([_, ..], [_, ..]) = (unreached.as_slice(), lacking.as_slice()) {
                    self.fetch_ids(&lacking)?;
                    let fetched = self.tip_commits(&tips)?;
                    for (at, commit) in unreached {
                        if self.reached(commit, &fetched)? {
                            held.insert(*at);
                        }
                    }
                }
            }
        }
        let (_, missing): (Vec<_>, Vec<_>) = revisions
            .iter()
            .enumerate()
            .partition(|(at, _)| held.contains(at));
        let missing: Vec<String> = missing
            .into_iter()
            .map(|(_, revision)| revision.clone())
            .collect();
        assert_eq!(
            missing.len() + held.len(),
            revisions.len(),
            "every revision is judged once"
        );
        Ok(missing)
    }

    fn commit_id_hex_len(&self) -> Result<usize, BoardStoreError> {
        let format = self
            .git
            .text(&["rev-parse", "--show-object-format"], None)
            .map_err(unavailable)?;
        match format.as_str() {
            "sha1" => Ok(40),
            "sha256" => Ok(64),
            other => Err(BoardStoreError::Unavailable(format!(
                "unknown object format {other:?}"
            ))),
        }
    }
}

impl GitRemoteRevisions {
    /// The objects the remote's branches and tags point at, once each.
    ///
    /// # Errors
    /// The remote cannot be listed, or git fails.
    fn remote_tips(&self) -> Result<Vec<String>, BoardStoreError> {
        let args = [
            "ls-remote",
            "--heads",
            "--tags",
            "--refs",
            self.remote.as_str(),
        ];
        let output = self
            .git
            .network_with(&args, None, MAX_TIP_LISTING)
            .map_err(unavailable)?;
        match output.status.success() {
            true => {}
            false => return Err(unavailable(failed(&args, &output))),
        }
        let mut tips: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|row| row.split_once('\t'))
            .map(|(oid, _)| object_id(oid))
            .collect::<Result<_, _>>()?;
        tips.sort_unstable();
        tips.dedup();
        Ok(tips)
    }

    /// Those of `tips` this clone does not hold.
    fn lacking<'t>(&self, tips: &'t [String]) -> Result<Vec<&'t String>, BoardStoreError> {
        Ok(tips
            .iter()
            .zip(self.batch_check(tips.iter().map(String::as_str))?)
            .filter_map(|(tip, found)| match found {
                Some(_) => None,
                None => Some(tip),
            })
            .collect())
    }

    /// The commits those of `tips` this clone holds point at (an annotated
    /// tag's commit); one that points at no commit (a tag of a tree, say)
    /// reaches none.
    fn tip_commits(&self, tips: &[String]) -> Result<Vec<String>, BoardStoreError> {
        let peeled: Vec<String> = tips.iter().map(|tip| format!("{tip}^{{commit}}")).collect();
        let commits = self.batch_check(peeled.iter().map(String::as_str))?;
        Ok(commits.into_iter().flatten().collect())
    }

    /// The object each of `names` names here, in order: `None` for one
    /// this clone does not hold (or that names no object of its kind).
    fn batch_check<'a>(
        &self,
        names: impl Iterator<Item = &'a str>,
    ) -> Result<Vec<Option<String>>, BoardStoreError> {
        let input: String = names.map(|name| format!("{name}\n")).collect();
        let args = ["cat-file", "--batch-check=%(objectname)"];
        let output = self
            .git
            .local(&args, Some(input.as_bytes()), MAX_TIP_LISTING)
            .map_err(unavailable)?;
        match output.status.success() {
            true => {}
            false => return Err(unavailable(failed(&args, &output))),
        }
        let found: Vec<Option<String>> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| object_id(line).ok())
            .collect();
        match found.len() == input.lines().count() {
            true => Ok(found),
            false => Err(BoardStoreError::Unavailable(
                "git did not answer for every object asked".into(),
            )),
        }
    }

    /// Fetches the objects `ids` from the remote, by id, moving no ref
    /// and writing no `FETCH_HEAD`.
    fn fetch_ids(&self, ids: &[&String]) -> Result<(), BoardStoreError> {
        let input: String = ids.iter().map(|id| format!("{id}\n")).collect();
        let args = [
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--recurse-submodules=no",
            "--stdin",
            self.remote.as_str(),
        ];
        let output = self
            .git
            .network_with(&args, Some(input.as_bytes()), SHORT_OUTPUT)
            .map_err(unavailable)?;
        match output.status.success() {
            true => Ok(()),
            false => Err(unavailable(failed(&args, &output))),
        }
    }

    /// Whether `commit` (a commit id or prefix) names a commit here that
    /// one of `tips` reaches.
    fn reached(&self, commit: &str, tips: &[String]) -> Result<bool, BoardStoreError> {
        let peeled = format!("{commit}^{{commit}}");
        let Some(Some(full)) = self.batch_check(std::iter::once(peeled.as_str()))?.pop() else {
            return Ok(false);
        };
        // Nothing of `full`'s history the tips do not reach: it is theirs
        // (with no tips, its own history is all left, so it is not).
        let input: String = std::iter::once(format!("{full}\n"))
            .chain(tips.iter().map(|tip| format!("^{tip}\n")))
            .collect();
        let args = ["rev-list", "-n", "1", "--stdin"];
        let output = self
            .git
            .local(&args, Some(input.as_bytes()), SHORT_OUTPUT)
            .map_err(unavailable)?;
        match output.status.success() {
            true => Ok(output.stdout.is_empty()),
            false => Err(unavailable(failed(&args, &output))),
        }
    }
}
