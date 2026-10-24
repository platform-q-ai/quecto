//! Full pinned rollback export. Structural traversal only; codecs stay in Reader.
use crate::application::project_board::ports::history::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub struct ExportBoardHistory {
    store: Arc<dyn BoardHistoryRead>,
    sink: Arc<dyn BoardHistorySink>,
}
impl ExportBoardHistory {
    pub fn new(store: Arc<dyn BoardHistoryRead>, sink: Arc<dyn BoardHistorySink>) -> Self {
        Self { store, sink }
    }
    pub fn execute_snapshot(
        &self,
        snapshot: &dyn BoardHistorySnapshot,
        dry: bool,
        expected: Option<&str>,
    ) -> HistoryResult<HistoryExportReport> {
        export(snapshot, self.sink.as_ref(), dry, expected)
    }
    pub fn execute(&self, dry: bool, expected: Option<&str>) -> HistoryResult<HistoryExportReport> {
        self.execute_snapshot(self.store.snapshot()?.as_ref(), dry, expected)
    }
}
fn malformed(s: &str) -> HistoryError {
    HistoryError::Malformed(s.into())
}
struct Traversal<'a> {
    view: &'a dyn BoardHistorySnapshot,
    sink: &'a dyn BoardHistorySink,
    dry: bool,
    // IDs only, never payloads. Eviction only causes pure recomputation.
    cache: BTreeMap<String, String>,
    path: BTreeSet<String>,
}
impl Traversal<'_> {
    fn object(
        &mut self,
        id: &str,
        depth: usize,
        original: bool,
        expected: HistoryKind,
    ) -> HistoryResult<String> {
        let o = self.view.object(id)?;
        if o.oid == id && (depth > 0 || o.kind == expected) {
        } else {
            return Err(malformed("object identity"));
        }
        let original = original || self.view.is_original(id)?;
        if o.kind == HistoryKind::Blob {
            if let Some(mapped) = self.cache.get(id) {
                return Ok(mapped.clone());
            }
        }
        let mapped = match o.kind {
            HistoryKind::Blob => self.sink.raw(
                o.kind,
                &o.bytes,
                if original { Some(id) } else { None },
                self.dry,
            )?,
            HistoryKind::Tree => {
                if self.path.insert(id.into()) {
                } else {
                    return Err(malformed("tree cycle"));
                }
                if depth <= 128 && o.bytes.len() <= 655360 && o.entries.len() <= 65536 {
                } else {
                    return Err(HistoryError::Bounds("export tree/depth".into()));
                }
                if matches!((o.bytes.is_empty(), original), (true, false)) {
                    let mut rows = o.entries;
                    for row in &mut rows {
                        if row.mode == "160000" {
                        } else {
                            row.oid = self.object(
                                &row.oid,
                                depth + 1,
                                false,
                                if row.mode == "040000" {
                                    HistoryKind::Tree
                                } else {
                                    HistoryKind::Blob
                                },
                            )?;
                        }
                    }
                    self.sink.tree(&rows, self.dry)?
                } else {
                    for row in &o.entries {
                        if row.mode == "160000" {
                        } else {
                            self.object(
                                &row.oid,
                                depth + 1,
                                true,
                                if row.mode == "040000" {
                                    HistoryKind::Tree
                                } else {
                                    HistoryKind::Blob
                                },
                            )?;
                        }
                    }
                    self.sink.raw(o.kind, &o.bytes, Some(id), self.dry)?
                }
            }
            _ => return Err(malformed("unexpected commit object")),
        };
        self.path.remove(id);
        if o.kind == HistoryKind::Blob {
            if self.cache.len() == 4096 {
                self.cache.pop_first();
            }
            self.cache.insert(id.into(), mapped.clone());
        }
        Ok(mapped)
    }
}
pub(super) fn export(
    snapshot: &dyn BoardHistorySnapshot,
    sink: &dyn BoardHistorySink,
    dry: bool,
    expected: Option<&str>,
) -> HistoryResult<HistoryExportReport> {
    let frontier = snapshot.frontier()?;
    if snapshot.algorithm()?.is_none() {
    } else {
        return Err(HistoryError::Unsupported(
            "imported provenance requires B".into(),
        ));
    }
    let mut traversal = Traversal {
        view: snapshot,
        sink,
        dry,
        cache: BTreeMap::new(),
        path: BTreeSet::new(),
    };
    let mut after = 0u64;
    let mut previous: Option<String> = None;
    while let Some(revision) = snapshot.next_revision(after)? {
        if revision.sequence
            == after
                .checked_add(1)
                .ok_or_else(|| malformed("revision overflow"))?
            && revision.parent == if after == 0 { None } else { Some(after) }
        {
        } else {
            return Err(malformed("noncontiguous lineage"));
        }
        let tree = traversal.object(
            &revision.root,
            0,
            revision.original.is_some(),
            HistoryKind::Tree,
        )?;
        let head = match revision.original {
            Some(original) => {
                if original.kind == HistoryKind::Commit {
                } else {
                    return Err(malformed("original revision kind"));
                }
                let header = original
                    .bytes
                    .split(|b| *b == b'\n')
                    .take_while(|line| !line.is_empty())
                    .collect::<Vec<_>>();
                let tree_line = format!("tree {tree}");
                let parents = header
                    .iter()
                    .filter_map(|line| line.strip_prefix(b"parent "))
                    .collect::<Vec<_>>();
                if header.first().copied() == Some(tree_line.as_bytes())
                    && match previous.as_deref() {
                        Some(p) => parents.as_slice() == [p.as_bytes()],
                        None => parents.is_empty(),
                    }
                {
                } else {
                    return Err(malformed("original lineage/root mismatch"));
                }
                sink.raw(
                    HistoryKind::Commit,
                    &original.bytes,
                    Some(&original.oid),
                    dry,
                )?
            }
            None => sink.commit(
                HistoryCommit {
                    tree: &tree,
                    parent: previous.as_deref(),
                    writer: &revision.writer,
                    message: &revision.message,
                    at: revision.at,
                },
                dry,
            )?,
        };
        previous = Some(head);
        after = revision.sequence;
    }
    let git_head = previous.ok_or_else(|| malformed("empty history"))?;
    if dry {
        sink.finish_dry_run()?;
    } else {
        sink.publish(expected, &git_head)?;
    }
    Ok(HistoryExportReport {
        revisions: after,
        frontier,
        git_head,
    })
}
