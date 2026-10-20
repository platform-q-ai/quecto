//! Which revisions the board's remote holds (#2666): what a run's end
//! judges a success by. A revision is held when it names a commit (its
//! full id, or a prefix of at least [`MIN_PREFIX`] hex digits) that one
//! of the remote's branches or tags reaches. The remote's tips are listed
//! (`ls-remote`), those this clone lacks are fetched by id (no ref, tag or
//! `FETCH_HEAD` moves), and each commit is walked against them here. A
//! commit this clone does not hold once every tip's history is here is
//! reached by none. Text that names no commit is never held, and asks
//! nothing of the remote. A shallow clone's walk stops at its boundary, so
//! a commit past it reads as not held: a success is then refused, never
//! recorded wrongly.
use super::*;
use crate::application::project_board::ports::RemoteRevisions;

/// The shortest prefix of a commit id taken as naming one, as git's own
/// default abbreviation.
const MIN_PREFIX: usize = 7;

/// The most a listing of the remote's branches and tags takes.
const MAX_TIP_LISTING: usize = 16 * 1024 * 1024;

/// `revision` as the commit id, or prefix of one, it spells, in lower
/// case: 7 to 64 hex digits (an allowlist), and nothing else.
fn commit_named(revision: &str) -> Option<String> {
    let hex = revision.bytes().all(|b| b.is_ascii_hexdigit());
    let long_enough = (MIN_PREFIX..=64).contains(&revision.len());
    (hex && long_enough).then(|| revision.to_ascii_lowercase())
}

impl RemoteRevisions for GitBoardStore {
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
                for (at, commit) in &named {
                    if self.reached(commit, &tips)? {
                        held.insert(*at);
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
}

impl GitBoardStore {
    /// The commits the remote's branches and tags point at (an annotated
    /// tag's commit), each held here once this returns: what the remote
    /// holds is what these reach.
    ///
    /// # Errors
    /// The remote cannot be listed or fetched from, or git fails.
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
        let lacking: Vec<&String> = tips
            .iter()
            .zip(self.batch_check(tips.iter().map(String::as_str))?)
            .filter_map(|(tip, found)| match found {
                Some(_) => None,
                None => Some(tip),
            })
            .collect();
        match lacking.as_slice() {
            [] => {}
            [_, ..] => self.fetch_ids(&lacking)?,
        }
        // Each tip as the commit it points at; one that points at no
        // commit (a tag of a tree, say) reaches none.
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
