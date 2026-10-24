//! Ordinary test-local port fakes for application orchestration. These do not
//! prove SQL/Git adapter behavior or provide a production observer/framework.
use super::project_board_history_fixture::*;
use quecto::application::project_board::ports::{BoardHead, history::*};
use quecto::composition::bootstrap::project_board_history::build_board_history_from_ports;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};
type Events = Arc<Mutex<Vec<String>>>;
struct View {
    history: Corpus,
    events: Events,
    committed: bool,
}
impl BoardHistorySnapshot for View {
    fn algorithm(&self) -> HistoryResult<Option<HistoryAlgorithm>> {
        Ok(None)
    }
    fn is_original(&self, _: &str) -> HistoryResult<bool> {
        Ok(false)
    }
    fn frontier(&self) -> HistoryResult<BoardHead> {
        Ok(BoardHead(format!("1:{}", self.history.revisions.len())))
    }
    fn next_revision(&self, after: u64) -> HistoryResult<Option<HistoryRevision>> {
        Ok(self
            .history
            .revisions
            .iter()
            .find(|r| r.sequence > after)
            .cloned())
    }
    fn object(&self, oid: &str) -> HistoryResult<HistoryObject> {
        self.history
            .objects
            .iter()
            .find(|o| o.oid == oid)
            .cloned()
            .ok_or_else(|| HistoryError::Malformed("test snapshot missing object".into()))
    }
}
impl Drop for View {
    fn drop(&mut self) {
        self.events.lock().unwrap().push(
            if self.committed {
                "drop-committed"
            } else {
                "drop-uncommitted"
            }
            .into(),
        );
    }
}
struct Store {
    history: Corpus,
    events: Events,
}
impl BoardHistoryRead for Store {
    fn snapshot(&self) -> HistoryResult<Box<dyn BoardHistorySnapshot>> {
        self.events.lock().unwrap().push("snapshot".into());
        Ok(Box::new(View {
            history: self.history.clone(),
            events: self.events.clone(),
            committed: false,
        }))
    }
}
struct Sink {
    repo: PathBuf,
    events: Events,
    fail_oid: Option<String>,
    ambiguous: bool,
}
impl BoardHistorySink for Sink {
    fn raw(
        &self,
        kind: HistoryKind,
        bytes: &[u8],
        expected: Option<&str>,
        dry_run: bool,
    ) -> HistoryResult<String> {
        let kind = match kind {
            HistoryKind::Blob => "blob",
            HistoryKind::Tree => "tree",
            HistoryKind::Commit => "commit",
        };
        let id = String::from_utf8(git(
            &self.repo,
            &["hash-object", "--literally", "-t", kind, "--stdin"],
            bytes,
        ))
        .unwrap()
        .trim()
        .to_string();
        assert_eq!(
            expected,
            Some(id.as_str()),
            "imported raw full-body hash verification"
        );
        self.events
            .lock()
            .unwrap()
            .push(format!("raw:{dry_run}:{id}"));
        if self.fail_oid.as_deref() == Some(id.as_str()) {
            return Err(HistoryError::Malformed(
                "injected original hash mismatch".into(),
            ));
        }
        Ok(id)
    }
    fn tree(&self, _: &[HistoryEntry], _: bool) -> HistoryResult<String> {
        Err(HistoryError::Unsupported(
            "only original raw tree expected".into(),
        ))
    }
    fn commit(&self, _: HistoryCommit<'_>, _: bool) -> HistoryResult<String> {
        Err(HistoryError::Unsupported(
            "only original raw commit expected".into(),
        ))
    }
    fn publish(&self, expected: Option<&str>, new_head: &str) -> HistoryResult<()> {
        self.events.lock().unwrap().push(format!(
            "publish:{}:{new_head}",
            expected.unwrap_or("absent")
        ));
        if self.ambiguous {
            Err(HistoryError::PublicationUnknown(format!(
                "expected={} new={new_head}",
                expected.unwrap_or("absent")
            )))
        } else {
            Ok(())
        }
    }
    fn finish_dry_run(&self) -> HistoryResult<()> {
        self.events.lock().unwrap().push("finish-dry-run".into());
        Ok(())
    }
}
fn graph(
    fixture: &Fixture,
    fail_oid: Option<String>,
    ambiguous: bool,
) -> (
    quecto::composition::bootstrap::project_board_history::BoardHistoryHandles,
    Events,
) {
    let events = Arc::new(Mutex::new(vec![]));
    let handles = build_board_history_from_ports(
        Arc::new(Store {
            history: fixture.history.clone(),
            events: events.clone(),
        }),
        Arc::new(Sink {
            repo: fixture.repo.clone(),
            events: events.clone(),
            fail_oid,
            ambiguous,
        }),
    );
    (handles, events)
}
fn all_originals_before(fixture: &Fixture, events: &[String], end: usize, dry: bool) {
    let expected: BTreeSet<_> = fixture
        .history
        .objects
        .iter()
        .map(|o| o.oid.clone())
        .chain(
            fixture
                .history
                .revisions
                .iter()
                .map(|r| r.original.as_ref().unwrap().oid.clone()),
        )
        .collect();
    let prefix = format!("raw:{dry}:");
    let actual: BTreeSet<_> = events[..end]
        .iter()
        .filter_map(|e| e.strip_prefix(&prefix).map(str::to_owned))
        .collect();
    assert_eq!(
        actual, expected,
        "all original kind+body hashes verified before publication/consuming commit"
    );
}
#[test]
fn feature_red_application_verifies_complete_snapshot_before_lease_and_reports_ambiguity() {
    let fixture = Fixture::new(HistoryAlgorithm::Sha1, 12);
    fixture.verify();
    let (handles, events) = graph(&fixture, None, true);
    let expected = "a".repeat(40);
    let result = handles.export.execute(false, Some(&expected));
    assert_eq!(
        result,
        Err(HistoryError::PublicationUnknown(format!(
            "expected={expected} new={}",
            fixture.history.source_head
        ))),
        "do not turn ambiguous publication into success/conflict"
    );
    let events = events.lock().unwrap();
    assert_eq!(events[0], "snapshot");
    let publish = events
        .iter()
        .position(|e| e.starts_with("publish:"))
        .unwrap();
    all_originals_before(&fixture, &events, publish, false);
    assert!(
        events
            .iter()
            .all(|e| e != "commit" && e != "finish-dry-run")
    );
    assert_eq!(
        events[publish],
        format!("publish:{expected}:{}", fixture.history.source_head)
    );
}
