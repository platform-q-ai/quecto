//! Retained-context hot-path ceilings (D9 #1978): how many retention-store
//! calls each sessions operation makes, recorded against the composed graph
//! over the real file store and pinned as decrease-only ceilings. A recall
//! of the index is one index read whatever the namespace holds (no
//! per-entry load); a recall of one entry is one read; retaining a tool
//! result is one append; retaining a conversation message is one index
//! read (the cached `Arc`) and one append — never an N+1 scan. The
//! baseline was recorded on master `c43dc0ba` before the move: the same
//! counts the direct store calls made.

use quecto::application::sessions::dto::retained_context::{RecallOutcome, RecallQuery};
use quecto::application::sessions::ports::{ContextSpillStore, SpillIndexList, SpillPresence};
use quecto::composition::retention::retention_handles_over;
use quecto::domain::error::DomainError;
use quecto::domain::session::SpillEntry;
use quecto::domain::session_identity::{SessionIdentity, SpillId};
use quecto::infrastructure::persistence::context_spill::FileContextSpillStore;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Decrease-only: store calls per operation (baseline == current).
const CEILING_INDEX_RECALL: usize = 1;
const CEILING_ENTRY_RECALL: usize = 1;
const CEILING_RETAIN_TOOL_RESULT: usize = 1;
const CEILING_RETAIN_CONVERSATION_MESSAGE: usize = 2;
const CEILING_PRESENCE: usize = 1;
/// Entries in the namespace the index recall is measured against: enough
/// that a per-entry load would be visible in the count.
const NAMESPACE_SIZE: usize = 2_000;

type Recall<'a> =
    Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + 'a>>;
type Unit<'a> = Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + 'a>>;

/// Counts every port call on its way to the real file store.
struct CountingStore {
    inner: FileContextSpillStore,
    calls: AtomicUsize,
}

impl CountingStore {
    fn new(base: &std::path::Path) -> Arc<Self> {
        Arc::new(Self {
            inner: FileContextSpillStore::new(FlatSessionLayout::new(base)),
            calls: AtomicUsize::new(0),
        })
    }
    fn take(&self) -> usize {
        self.calls.swap(0, Ordering::SeqCst)
    }
    fn tick(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

impl ContextSpillStore for CountingStore {
    fn append(&self, identity: &SessionIdentity, entry: &SpillEntry) -> Unit<'_> {
        self.tick();
        self.inner.append(identity, entry)
    }
    fn recall(&self, identity: &SessionIdentity, id: &SpillId) -> Recall<'_> {
        self.tick();
        self.inner.recall(identity, id)
    }
    fn list_entries(&self, identity: &SessionIdentity) -> SpillIndexList<'_> {
        self.tick();
        self.inner.list_entries(identity)
    }
    fn has_entries<'a>(&'a self, identity: &'a SessionIdentity) -> SpillPresence<'a> {
        self.tick();
        self.inner.has_entries(identity)
    }
    fn clear(&self, identity: &SessionIdentity) -> Unit<'_> {
        self.tick();
        self.inner.clear(identity)
    }
    fn scrub_sync(&self, identity: &SessionIdentity) {
        self.tick();
        self.inner.scrub_sync(identity)
    }
}

fn entry(id: &str) -> SpillEntry {
    SpillEntry {
        id: id.to_string(),
        tool: "bash".to_string(),
        input_preview: "ls".to_string(),
        tokens: 3,
        content: format!("content of {id}"),
    }
}

#[tokio::test]
async fn every_retained_context_operation_stays_under_its_store_call_ceiling() {
    let tmp = tempfile::tempdir().unwrap();
    let store = CountingStore::new(tmp.path());
    let handles = retention_handles_over(store.clone());
    let identity = SessionIdentity::named_cli("scale").unwrap();

    // Seed a large namespace through the writer: one append each.
    for i in 0..NAMESPACE_SIZE {
        handles
            .context
            .retain
            .retain(&identity, &entry(&format!("turn{i}:bash:0")))
            .await
            .unwrap();
        assert_eq!(store.take(), CEILING_RETAIN_TOOL_RESULT);
    }

    // Cold and warm index recalls: one index read each, never a per-entry load.
    for _ in 0..2 {
        let started = std::time::Instant::now();
        let outcome = handles
            .recall
            .recall(&identity, &RecallQuery::Index)
            .await
            .unwrap();
        let elapsed = started.elapsed();
        let RecallOutcome::Index(entries) = outcome else {
            panic!("index");
        };
        assert_eq!(entries.len(), NAMESPACE_SIZE);
        assert!(store.take() <= CEILING_INDEX_RECALL);
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "index recall of {NAMESPACE_SIZE} entries took {elapsed:?}"
        );
    }

    // One entry: one read, known or unknown.
    let known = handles
        .recall
        .recall(
            &identity,
            &RecallQuery::Entry(SpillId::new("turn1999:bash:0")),
        )
        .await
        .unwrap();
    assert!(matches!(known, RecallOutcome::Entry(e) if e.content == "content of turn1999:bash:0"));
    assert!(store.take() <= CEILING_ENTRY_RECALL);
    let missing = handles
        .recall
        .recall(&identity, &RecallQuery::Entry(SpillId::new("turn9:none:9")))
        .await
        .unwrap();
    assert!(matches!(missing, RecallOutcome::Missing(_)));
    assert!(store.take() <= CEILING_ENTRY_RECALL);

    // A conversation message: one index read and one append, twice over
    // (the second collides and takes `:2`).
    for expected in ["turn1:msg:assistant", "turn1:msg:assistant:2"] {
        let mut e = entry("turn1:msg:assistant");
        let receipt = handles
            .context
            .retain
            .retain_deduplicated(&identity, &mut e)
            .await
            .unwrap();
        assert_eq!(receipt.id, expected);
        assert!(store.take() <= CEILING_RETAIN_CONVERSATION_MESSAGE);
    }

    // Presence: one call.
    assert!(
        handles
            .context
            .list
            .retains_entries(&identity)
            .await
            .unwrap()
    );
    assert!(store.take() <= CEILING_PRESENCE);
}
