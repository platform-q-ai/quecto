//! Written export objects, staged on disk and stored by git in batches (#2748).
use super::*;

/// Objects stored by one `git hash-object --stdin-paths` call, at most.
pub(crate) const WRITE_BATCH: usize = 4096;
/// Bytes of objects staged for one call, at most, unless one object alone
/// is larger (a blob may be 64 MiB, the largest any one object may be).
pub(crate) const WRITE_BATCH_BYTES: usize = 64 * 1024 * 1024;

/// Objects hashed here and not yet stored by git: one file each, named by
/// its ID, in a private directory removed when the batch is stored.
#[derive(Default)]
pub(super) struct Staged {
    dir: Option<tempfile::TempDir>,
    ids: BTreeMap<&'static str, Vec<String>>,
    seen: BTreeSet<String>,
    bytes: usize,
}

impl Staged {
    /// Whether every staged object has been stored.
    pub(super) fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

impl GitBoardHistorySink {
    /// Stages `bytes`, already hashed to `id`, storing the batch first when
    /// it is full or `bytes` would take it past its byte bound.
    pub(super) fn stage(&self, k: HistoryKind, bytes: &[u8], id: &str) -> HistoryResult<()> {
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
        let dir = match staged.dir.take() {
            Some(dir) => dir,
            None => tempfile::Builder::new()
                .prefix("quecto-export-")
                .tempdir()
                .map_err(unavailable)?,
        };
        let written = std::fs::write(dir.path().join(id), bytes).map_err(unavailable);
        staged.dir = Some(dir);
        written?;
        staged.ids.entry(kind(k)).or_default().push(id.into());
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

    /// Has git store every staged object, each under the ID hashed here.
    /// The batch is dropped whatever happens: a failed export or store
    /// leaves only objects nothing names.
    pub(super) fn store(&self, staged: &mut Staged) -> HistoryResult<()> {
        let Staged { dir, ids, seen, .. } = std::mem::take(staged);
        let Some(dir) = dir else {
            assert!(seen.is_empty(), "staged objects have a directory");
            return Ok(());
        };
        let git = git(&self.repo)?;
        for (name, ids) in &ids {
            let mut input = Vec::new();
            for id in ids {
                let path = dir.path().join(id);
                input.extend_from_slice(listable(&path)?.as_bytes());
                input.push(b'\n');
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
                name,
                "--stdin-paths",
            ];
            let out = git
                .ok_up_to(&args, Some(&input), limit)
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

/// `path` as a line `hash-object --stdin-paths` reads back unchanged: an
/// absolute path of visible characters and spaces alone (no line breaks,
/// no leading quote that git would unquote).
pub(crate) fn listable(path: &std::path::Path) -> HistoryResult<&str> {
    match path.to_str() {
        Some(text)
            if text.starts_with('/')
                && text
                    .chars()
                    .all(|c| c == ' ' || c.is_ascii_graphic() || c.is_alphanumeric()) =>
        {
            Ok(text)
        }
        _ => Err(unavailable("export staging path is not a plain line")),
    }
}
