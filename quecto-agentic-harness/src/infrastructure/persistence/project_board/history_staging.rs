//! Written export objects, staged on disk and stored by git in batches (#2748).
use super::*;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Objects stored by one `git hash-object --stdin-paths` call, at most.
pub(crate) const WRITE_BATCH: usize = 4096;
/// Bytes of objects staged for one call, at most, unless one object alone
/// is larger (a blob may be 64 MiB, the largest any one object may be).
pub(crate) const WRITE_BATCH_BYTES: usize = 64 * 1024 * 1024;
/// Kinds in the order a batch stores them: what a tree or commit names is
/// stored before it, so a stored commit always has its tree.
const STORE_ORDER: [HistoryKind; 3] = [HistoryKind::Blob, HistoryKind::Tree, HistoryKind::Commit];

/// Objects hashed here and not yet stored by git: one file each, named by
/// its ID, in a private directory removed when the batch is stored.
#[derive(Default)]
pub(super) struct Staged {
    /// The directory and its path as the line git is given, checked once.
    dir: Option<(tempfile::TempDir, String)>,
    /// IDs by kind, in [`STORE_ORDER`].
    ids: [Vec<String>; 3],
    seen: BTreeSet<String>,
    bytes: usize,
}

impl Staged {
    /// Whether every staged object has been stored.
    pub(super) fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
    /// Staged IDs, in the order a store would give them to git.
    #[cfg(test)]
    pub(super) fn order(&self) -> Vec<String> {
        self.ids.concat()
    }
    /// Where a staged object's bytes wait for git.
    #[cfg(test)]
    pub(super) fn file(&self, id: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|(dir, _)| dir.path().join(id))
    }
}

fn slot(k: HistoryKind) -> usize {
    match k {
        HistoryKind::Blob => 0,
        HistoryKind::Tree => 1,
        HistoryKind::Commit => 2,
    }
}

/// The deadline of one store call: git's ordinary local deadline, plus
/// 10 ms an object and 1 s a MiB, so a full batch (4,096 objects, 64 MiB)
/// gets about 135 s. Past it the call fails, and nothing is published.
pub(crate) fn batch_timeout(objects: usize, bytes: usize) -> Duration {
    let objects = u64::try_from(objects).unwrap_or(u64::MAX);
    let mebibytes = u64::try_from(bytes / (1024 * 1024) + 1).unwrap_or(u64::MAX);
    super::super::git::LOCAL_TIMEOUT
        .saturating_add(Duration::from_millis(objects.saturating_mul(10)))
        .saturating_add(Duration::from_secs(mebibytes))
}

impl GitBoardHistorySink {
    /// Stages `bytes`, already hashed to `id`, storing the batch first when
    /// it is full or `bytes` would take it past its byte bound. Any failure
    /// marks the sink failed: it then never publishes.
    pub(super) fn stage(&self, k: HistoryKind, bytes: &[u8], id: &str) -> HistoryResult<()> {
        let result = self.stage_unmarked(k, bytes, id);
        if result.is_err() {
            self.failed.store(true, Ordering::SeqCst);
        }
        result
    }

    /// Stores everything staged, before publication moves the ref. Refuses
    /// once any earlier store failed: objects the head may reach are lost.
    pub(super) fn store_for_publish(&self) -> HistoryResult<()> {
        if self.failed.load(Ordering::SeqCst) {
            return Err(unavailable(
                "an earlier object store into the destination failed, so this export \
                 cannot publish; export again",
            ));
        }
        let mut staged = self.staged.lock().map_err(unavailable)?;
        let stored = self.store(&mut staged);
        if stored.is_err() {
            self.failed.store(true, Ordering::SeqCst);
        }
        stored?;
        assert!(staged.is_empty(), "nothing staged is left unstored");
        Ok(())
    }

    /// git for one store call of `objects` objects, `bytes` long in all,
    /// with that batch's own deadline ([`batch_timeout`]).
    pub(super) fn store_git(&self, objects: usize, bytes: usize) -> HistoryResult<Git> {
        let mut git = git(&self.repo)?;
        git.local_timeout = batch_timeout(objects, bytes);
        Ok(git)
    }

    fn stage_unmarked(&self, k: HistoryKind, bytes: &[u8], id: &str) -> HistoryResult<()> {
        let mut staged = self.staged.lock().map_err(unavailable)?;
        if staged.seen.contains(id) {
            return Ok(());
        }
        let full = staged.seen.len() == WRITE_BATCH
            || staged
                .bytes
                .checked_add(bytes.len())
                .is_none_or(|n| n > WRITE_BATCH_BYTES);
        if full && staged.dir.is_some() {
            self.store(&mut staged)?;
        }
        let (dir, line) = match staged.dir.take() {
            Some(dir) => dir,
            None => self.staging_dir()?,
        };
        let written = std::fs::write(dir.path().join(id), bytes).map_err(unavailable);
        staged.dir = Some((dir, line));
        written?;
        staged.ids[slot(k)].push(id.into());
        staged.seen.insert(id.into());
        staged.bytes = staged
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| bounds("staged bytes overflow"))?;
        assert!(
            staged.seen.len() <= WRITE_BATCH,
            "a full batch is stored before another object is staged"
        );
        Ok(())
    }

    /// A private (0700) directory in the destination's own git directory,
    /// so a crash leaves it with the repository, on its filesystem; its
    /// path checked once as a line git reads back unchanged. Old ones a
    /// crashed export left there are swept first.
    fn staging_dir(&self) -> HistoryResult<(tempfile::TempDir, String)> {
        let git_dir = self.git_dir()?;
        sweep_leftovers(&git_dir);
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .prefix(STAGING_PREFIX)
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(&git_dir)
            .map_err(|e| {
                unavailable(format!(
                    "cannot stage export objects in the destination git directory \
                     {git_dir:?}: {e}"
                ))
            })?;
        let line = listable(dir.path())
            .map_err(|_| {
                unavailable(format!(
                    "the destination git directory {git_dir:?} is not one absolute \
                     UTF-8 line that git can read back, so export objects cannot be \
                     staged there"
                ))
            })?
            .to_owned();
        Ok((dir, line))
    }

    /// Has git store every staged object, kind by kind in [`STORE_ORDER`],
    /// each under the ID hashed here. The batch is dropped whatever happens:
    /// a failed store leaves only objects nothing published names.
    fn store(&self, staged: &mut Staged) -> HistoryResult<()> {
        let Staged {
            dir, ids, bytes, ..
        } = std::mem::take(staged);
        let Some((dir, line)) = dir else {
            assert!(
                ids.iter().all(Vec::is_empty),
                "staged objects have a directory"
            );
            return Ok(());
        };
        let git = self.store_git(ids.iter().map(Vec::len).sum(), bytes)?;
        for (k, ids) in STORE_ORDER.into_iter().zip(&ids) {
            if ids.is_empty() {
                continue;
            }
            let mut input = String::new();
            for id in ids {
                assert!(
                    id.bytes().all(|b| b.is_ascii_hexdigit()),
                    "a staged file is named by its hex ID"
                );
                input.push_str(&format!("{line}/{id}\n"));
            }
            let limit = ids
                .len()
                .checked_mul(65)
                .ok_or_else(|| bounds("batch output"))?;
            let args = [
                "hash-object",
                "--literally",
                "--no-filters",
                "-w",
                "-t",
                kind(k),
                "--stdin-paths",
            ];
            let out = git
                .ok_up_to(&args, Some(input.as_bytes()), limit)
                .map_err(unavailable)?;
            let written = std::str::from_utf8(&out).map_err(malformed)?;
            if written.lines().eq(ids.iter().map(String::as_str)) {
            } else {
                return Err(malformed("written hash mismatch"));
            }
        }
        dir.close().map_err(unavailable)
    }
}

/// How long a staging directory goes unchanged before it is taken for a
/// crashed export's: a live export changes its own with every object.
const LEFTOVER_AGE: Duration = Duration::from_secs(3600);
/// Name prefix of every staging directory an export makes.
const STAGING_PREFIX: &str = "quecto-export-";

/// Removes, from `git_dir`, staging directories (and only those: a real
/// directory named `quecto-export-*`) unchanged for [`LEFTOVER_AGE`].
/// Called before this export makes its own, so its own is never one.
/// Best effort: a directory that cannot be read or removed is left for the
/// next export's sweep, and never fails this one. It is not logged: the
/// board's persistence modules write no `tracing` record (an architecture
/// rule, `project_board_telemetry`).
fn sweep_leftovers(git_dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(git_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let ours = name
            .to_str()
            .is_some_and(|name| name.starts_with(STAGING_PREFIX));
        // Not followed: a link named like ours is never a directory here.
        let old = entry.metadata().is_ok_and(|meta| {
            meta.file_type().is_dir()
                && meta
                    .modified()
                    .ok()
                    .and_then(|at| at.elapsed().ok())
                    .is_some_and(|age| age >= LEFTOVER_AGE)
        });
        if ours && old {
            let _left_for_the_next_sweep = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// `path` as a line `hash-object --stdin-paths` reads back unchanged:
/// absolute UTF-8 that is one whole line, ending in neither a line feed nor
/// a carriage return (git strips a CR before the LF). Starting with `/`, it
/// never starts with the `"` git would unquote.
pub(crate) fn listable(path: &std::path::Path) -> HistoryResult<&str> {
    match path.to_str() {
        Some(text) if text.starts_with('/') && text.split(['\n', '\r']).next() == Some(text) => {
            Ok(text)
        }
        _ => Err(unavailable("export staging path is not one absolute line")),
    }
}
