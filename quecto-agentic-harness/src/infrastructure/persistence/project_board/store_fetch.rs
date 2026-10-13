//! Fetching the board (#2481 S10b): its commits into the tracking ref, and
//! in a partial clone what a read of them reads, one tree level per round.
//!
//! In a clone whose promisor is the board's remote, the board's commits
//! come alone (`--filter=tree:0`); the root and the archive as their trees
//! alone ([`Wanted::Alone`]), so nothing under the archive is ever fetched;
//! and each live folder whole but for any blob past every file a read
//! reads ([`Wanted::Filtered`], `--filter=blob:limit=`[`FILTER_BYTES`]),
//! which the read sets aside and a write carries by its id. A clone whose
//! remote is no promisor fetches with `--no-filter`: a filter there would
//! make it a partial clone.
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::{
    FILTER_BYTES, Unread, Wanted, read_board,
};

/// Fetch rounds of what a partial clone misses, one per tree level: the
/// root, then the folders and the archive, then (for a clone that holds
/// the trees but not their files) the folders' files.
const FETCH_ROUNDS: usize = 3;

impl GitBoardStore {
    /// The board at `commit`, fetching in one round what this clone does
    /// not hold yet (a partial clone).
    pub(super) fn read_fetching(&self, commit: &str) -> Result<Board, BoardStoreError> {
        let mut fetched: BTreeSet<Wanted> = BTreeSet::new();
        // The trees fetched with the blob filter: a blob still missing
        // under one is past every cap.
        let mut filtered: BTreeSet<String> = BTreeSet::new();
        for round in 0..=FETCH_ROUNDS {
            let missing = match read_board(&self.git, commit, self.limits, &filtered) {
                Ok(board) => return Ok(board),
                Err(Unread::Failed(BoardStoreError::Clash(why))) => {
                    return Err(BoardStoreError::Clash(format!(
                        "`quecto/board` on {} is a branch of its own: {why}; rename it",
                        self.remote
                    )));
                }
                Err(Unread::Failed(error)) => return Err(error),
                Err(Unread::Missing(missing)) => missing,
            };
            assert!(!missing.is_empty(), "a read misses at least one object");
            let again = missing.iter().any(|wanted| fetched.contains(wanted));
            if again || round == FETCH_ROUNDS {
                let oids: Vec<&str> = missing.iter().map(oid_of).collect();
                return Err(BoardStoreError::Unavailable(format!(
                    "objects of board commit {commit} are missing here and were not fetched: {}",
                    oids.join(" ")
                )));
            }
            self.fetch_objects(&missing)?;
            filtered.extend(missing.iter().filter_map(|wanted| match wanted {
                Wanted::Filtered(oid) => Some(oid.clone()),
                Wanted::Alone(_) => None,
            }));
            fetched.extend(missing);
        }
        unreachable!("the last round returns")
    }

    /// Fetches what `wanted` names: in a partial clone, each tree alone or
    /// filtered as it says, one call for each way; otherwise all of them
    /// and everything under them in one call, as git's own lazy fetch
    /// does, but with every file. A tree this clone holds is fetched again
    /// with what lies under it (`--refetch`): git skips a want it holds.
    fn fetch_objects(&self, wanted: &[Wanted]) -> Result<(), BoardStoreError> {
        let blobs = format!("--filter=blob:limit={FILTER_BYTES}");
        let calls: Vec<(&str, Vec<&str>)> = match self.promisor()? {
            true => {
                let alone = wanted.iter().filter(|w| matches!(w, Wanted::Alone(_)));
                let whole = wanted.iter().filter(|w| matches!(w, Wanted::Filtered(_)));
                vec![
                    ("--filter=tree:0", alone.map(oid_of).collect()),
                    (blobs.as_str(), whole.map(oid_of).collect()),
                ]
            }
            false => vec![("--no-filter", wanted.iter().map(oid_of).collect())],
        };
        for (filter, oids) in calls.iter().filter(|(_, oids)| !oids.is_empty()) {
            let args = [
                "-c",
                "fetch.negotiationAlgorithm=noop",
                "fetch",
                "--quiet",
                "--no-tags",
                "--no-write-fetch-head",
                "--no-recurse-submodules",
                "--refetch",
                filter,
                "--stdin",
                &self.remote,
            ];
            let input: String = oids.iter().map(|oid| format!("{oid}\n")).collect();
            let output = self
                .git
                .network_with(&args, Some(input.as_bytes()), SHORT_OUTPUT)
                .map_err(unavailable)?;
            if !output.status.success() {
                return Err(BoardStoreError::Unavailable(format!(
                    "objects missing here could not be fetched from {}: {}: {}",
                    self.remote,
                    oids.join(" "),
                    failed(&args, &output)
                )));
            }
        }
        Ok(())
    }

    /// Fetches the board alone into the tracking ref (no tags, no
    /// `FETCH_HEAD`): in a partial clone its commits alone, otherwise with
    /// all its files; its commit.
    pub(super) fn fetch(&self) -> Result<String, BoardStoreError> {
        let spec = format!("+{BOARD_REF}:{}", self.tracking());
        let filter = match self.promisor()? {
            true => "--filter=tree:0",
            false => "--no-filter",
        };
        let args = [
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            filter,
            &self.remote,
            &spec,
        ];
        let output = self.git.network(&args).map_err(unavailable)?;
        if !output.status.success() {
            return Err(unavailable(failed(&args, &output)));
        }
        let commit = format!("{}^{{commit}}", self.tracking());
        object_id(
            &self
                .git
                .text(&["rev-parse", "--verify", "--quiet", &commit], None)
                .map_err(unavailable)?,
        )
    }

    /// Whether the board's remote is this clone's promisor, so a fetch from
    /// it may leave objects there: `remote.<name>.promisor`, or the older
    /// `extensions.partialClone` naming it. Asked once.
    fn promisor(&self) -> Result<bool, BoardStoreError> {
        if let Some(promisor) = self.promisor.get() {
            return Ok(*promisor);
        }
        let flag = format!("remote.{}.promisor", self.remote);
        let flagged = self.config(&["config", "--type=bool", "--get", &flag])?;
        let named = self.config(&["config", "--get", "extensions.partialClone"])?;
        let promisor = flagged.as_deref() == Some("true") || named.as_deref() == Some(&self.remote);
        Ok(*self.promisor.get_or_init(|| promisor))
    }

    /// A configured value, if it is set.
    fn config(&self, args: &[&str]) -> Result<Option<String>, BoardStoreError> {
        let output = self
            .git
            .local(args, None, SHORT_OUTPUT)
            .map_err(unavailable)?;
        match output.status.code() {
            Some(0) => Ok(Some(
                String::from_utf8_lossy(&output.stdout)
                    .trim_end()
                    .to_string(),
            )),
            Some(1) => Ok(None),
            _ => Err(unavailable(failed(args, &output))),
        }
    }
}

/// The object `wanted` names.
fn oid_of(wanted: &Wanted) -> &str {
    match wanted {
        Wanted::Alone(oid) | Wanted::Filtered(oid) => oid,
    }
}
