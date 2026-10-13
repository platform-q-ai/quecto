//! Fetching the board (#2481 S10b): its commits into the tracking ref, and
//! in a partial clone what a read of them reads, one tree level per round.
//!
//! In a clone whose promisor is the board's remote:
//! - a new head comes as what changed since the last board read whole
//!   (`store_fetch_state`), every file but those past every file a read
//!   reads (`--filter=blob:limit=`[`FILTER_BYTES`], negotiated from that
//!   commit): one fetch, as a full clone's;
//! - with no such commit, the board's commits come alone
//!   (`--filter=tree:0`), then the root and the archive as their trees
//!   alone ([`Wanted::Alone`]), nothing under the archive, and each live
//!   folder whole but for the files past every cap ([`Wanted::Filtered`]);
//! - a file a filter left on the remote is set aside, carried by its id and
//!   remembered, so no later read fetches its folder whole again.
//!
//! Every filtered fetch passes the clone's own fetch filter on (`-c
//! remote.<name>.partialclonefilter`), so git never records ours as the
//! clone's. A server that refuses filters, or knows none, is fetched from
//! whole. A clone whose remote is no promisor fetches with `--no-filter`
//! and never `--refetch`: a filter there would make it a partial clone, and
//! a file it lacks is no file past a cap but a clone to mend.
use super::fetch_state::FetchState;
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::{
    FILTER_BYTES, LeftOnRemote, Unread, Wanted, read_board,
};

/// Fetch rounds of what a partial clone misses, one per tree level: the
/// root, then the folders and the archive, then (for a clone that holds
/// the trees but not their files) the folders' files.
const FETCH_ROUNDS: usize = 3;
/// What git says of a server that knows no filters, fetching whole.
const NO_FILTERS: &[u8] = b"filtering not recognized by server";

/// Whether the board's remote is this clone's promisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Partial {
    /// A full clone, or one whose promisor is another remote.
    Full,
    /// A partial clone of the board's remote, with its own fetch filter for
    /// it, if it sets one.
    Promisor { filter: Option<String> },
}

/// How the objects of a fetch came.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Came {
    Filtered,
    Whole,
}

impl GitBoardStore {
    /// The board at `commit`, fetching in one round what this clone does
    /// not hold yet (a partial clone).
    pub(super) fn read_fetching(&self, commit: &str) -> Result<Board, BoardStoreError> {
        let promisor = matches!(self.partial()?, Partial::Promisor { .. });
        let before = match promisor {
            true => self.fetch_state(),
            false => FetchState::default(),
        };
        let mut fetched: BTreeSet<Wanted> = BTreeSet::new();
        let mut left = LeftOnRemote {
            trees: BTreeSet::new(),
            blobs: before.omitted.clone(),
        };
        for round in 0..=FETCH_ROUNDS {
            let missing = match read_board(&self.git, commit, self.limits, &left) {
                Ok(board) => {
                    if promisor {
                        let read = FetchState {
                            read: Some(commit.to_string()),
                            omitted: board.omitted.clone(),
                        };
                        self.keep_fetch_state(&before, &read);
                    }
                    return Ok(board);
                }
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
            // Only a promisor's filtered fetch leaves a file on the remote.
            left.trees.extend(self.fetch_objects(&missing)?);
            fetched.extend(missing);
        }
        unreachable!("the last round returns")
    }

    /// Fetches what `wanted` names; the trees whose files came filtered.
    /// In a partial clone, each tree alone or filtered as it says, one
    /// call for each way, a tree it holds fetched again with what lies
    /// under it (`--refetch`: git skips a want it holds). Otherwise all of
    /// them and everything under them in one call, as git's own lazy fetch
    /// does, but with every file.
    fn fetch_objects(&self, wanted: &[Wanted]) -> Result<Vec<String>, BoardStoreError> {
        let blobs = format!("--filter=blob:limit={FILTER_BYTES}");
        let oids = |alone: bool| -> Vec<&str> {
            let picked = wanted
                .iter()
                .filter(|w| matches!(w, Wanted::Alone(_)) == alone);
            picked.map(oid_of).collect()
        };
        let calls: Vec<(Option<&str>, Vec<&str>)> = match self.partial()? {
            Partial::Promisor { .. } => vec![
                (Some("--filter=tree:0"), oids(true)),
                (Some(blobs.as_str()), oids(false)),
            ],
            Partial::Full => vec![(None, wanted.iter().map(oid_of).collect())],
        };
        let mut filtered = vec![];
        for (filter, oids) in calls.iter().filter(|(_, oids)| !oids.is_empty()) {
            let refetch: &[&str] = match filter {
                Some(_) => &["--refetch"],
                None => &[],
            };
            let options = [
                &["--no-recurse-submodules"][..],
                refetch,
                &["--stdin", &self.remote],
            ]
            .concat();
            let input: String = oids.iter().map(|oid| format!("{oid}\n")).collect();
            let config = ["fetch.negotiationAlgorithm=noop"];
            let came = self
                .fetching(&config, &options, *filter, Some(input.as_bytes()))
                .map_err(|why| {
                    BoardStoreError::Unavailable(format!(
                        "objects missing here could not be fetched from {}: {}: {why}",
                        self.remote,
                        oids.join(" ")
                    ))
                })?;
            if (came, *filter) == (Came::Filtered, Some(blobs.as_str())) {
                filtered.extend(oids.iter().map(|oid| (*oid).to_string()));
            }
        }
        Ok(filtered)
    }

    /// Fetches the board alone into the tracking ref (no tags, no
    /// `FETCH_HEAD`): in a partial clone what changed since the last board
    /// read whole, or with none its commits alone; otherwise with all its
    /// files. Its commit.
    pub(super) fn fetch(&self) -> Result<String, BoardStoreError> {
        let spec = format!("+{BOARD_REF}:{}", self.tracking());
        let blobs = format!("--filter=blob:limit={FILTER_BYTES}");
        let tip = match self.partial()? {
            Partial::Promisor { .. } => self.fetch_state().read,
            Partial::Full => None,
        };
        let tip = match tip {
            Some(tip) if self.is_commit(&tip)? => Some(format!("--negotiation-tip={tip}")),
            Some(_) | None => None,
        };
        let filter = match (self.partial()?, &tip) {
            (Partial::Full, _) => None,
            (Partial::Promisor { .. }, Some(_)) => Some(blobs.as_str()),
            (Partial::Promisor { .. }, None) => Some("--filter=tree:0"),
        };
        let mut options = vec!["--no-recurse-submodules"];
        options.extend(tip.as_deref());
        options.extend([self.remote.as_str(), spec.as_str()]);
        let config = ["fetch.negotiationAlgorithm=consecutive"];
        self.fetching(&config, &options, filter, None)
            .map_err(BoardStoreError::Unavailable)?;
        let commit = format!("{}^{{commit}}", self.tracking());
        object_id(
            &self
                .git
                .text(&["rev-parse", "--verify", "--quiet", &commit], None)
                .map_err(unavailable)?,
        )
    }

    /// Runs `git fetch` with `config`, `options` and `filter` (none: every
    /// file). A filter is passed with the clone's own fetch filter, and a
    /// server that refuses it is fetched from again whole. How the objects
    /// came, or what git said.
    fn fetching(
        &self,
        config: &[&str],
        options: &[&str],
        filter: Option<&str>,
        input: Option<&[u8]>,
    ) -> Result<Came, String> {
        let own = match (self.partial(), filter) {
            (Ok(Partial::Promisor { filter: own }), Some(ours)) => {
                let kept = own.unwrap_or_else(|| ours.trim_start_matches("--filter=").into());
                Some(format!("remote.{}.partialclonefilter={kept}", self.remote))
            }
            (Ok(_), _) => None,
            (Err(error), _) => return Err(told(&error).to_string()),
        };
        let run = |filter: &str| -> Result<super::super::git::Output, String> {
            let mut args: Vec<&str> = vec![];
            for setting in config.iter().copied().chain(own.as_deref()) {
                args.extend(["-c", setting]);
            }
            args.extend([
                "fetch",
                "--quiet",
                "--no-tags",
                "--no-write-fetch-head",
                filter,
            ]);
            args.extend(options);
            let output = self
                .git
                .network_with(&args, input, SHORT_OUTPUT)
                .map_err(|error| error.to_string())?;
            match output.status.success() {
                true => Ok(output),
                false => Err(failed(&args, &output).to_string()),
            }
        };
        let Some(filter) = filter else {
            return run("--no-filter").map(|_| Came::Whole);
        };
        match run(filter) {
            Ok(output) if contains(&output.stderr, NO_FILTERS) => Ok(Came::Whole),
            Ok(_) => Ok(Came::Filtered),
            // A server that refuses this filter: `filter '<kind>' not
            // supported`.
            Err(why) if why.contains("filter '") && why.contains("' not supported") => {
                run("--no-filter").map(|_| Came::Whole)
            }
            Err(why) => Err(why),
        }
    }

    /// Whether the board's remote is this clone's promisor, so a fetch from
    /// it may leave objects there (`remote.<name>.promisor`, or the older
    /// `extensions.partialClone` naming it), and the clone's own fetch
    /// filter for it. Asked once.
    pub(super) fn partial(&self) -> Result<Partial, BoardStoreError> {
        if let Some(partial) = self.partial.get() {
            return Ok(partial.clone());
        }
        let flag = format!("remote.{}.promisor", self.remote);
        let flagged = self.config(&["config", "--type=bool", "--get", &flag])?;
        let named = self.config(&["config", "--get", "extensions.partialClone"])?;
        let partial =
            match flagged.as_deref() == Some("true") || named.as_deref() == Some(&self.remote) {
                true => {
                    let key = format!("remote.{}.partialclonefilter", self.remote);
                    let filter = self.config(&["config", "--get", &key])?;
                    Partial::Promisor { filter }
                }
                false => Partial::Full,
            };
        Ok(self.partial.get_or_init(|| partial).clone())
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

/// Whether `bytes` hold `needle`.
fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

/// The object `wanted` names.
fn oid_of(wanted: &Wanted) -> &str {
    match wanted {
        Wanted::Alone(oid) | Wanted::Filtered(oid) => oid,
    }
}
