//! Test rig for the retained-context use cases (D9 #1978): an in-memory
//! retention store that journals every port call it receives, keyed by the
//! typed identity, with switchable failures per operation.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::application::sessions::ports::{ContextSpillStore, SpillIndexList, SpillPresence};
use crate::domain::error::DomainError;
use crate::domain::session::{SpillEntry, SpillIndex};
use crate::domain::session_identity::{SessionIdentity, SpillId};

type Recall<'a> =
    Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + 'a>>;
type Unit<'a> = Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + 'a>>;

#[derive(Debug, Default)]
pub(crate) struct JournalingRetention {
    entries: Mutex<Vec<(SessionIdentity, SpillEntry)>>,
    journal: Mutex<Vec<String>>,
    pub(crate) fail_append: Mutex<bool>,
    pub(crate) fail_recall: Mutex<bool>,
    pub(crate) fail_list: Mutex<bool>,
    pub(crate) fail_clear: Mutex<bool>,
}

impl JournalingRetention {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn seeded(identity: &SessionIdentity, ids: &[&str]) -> Arc<Self> {
        let rig = Self::new();
        rig.entries.lock().unwrap().extend(
            ids.iter()
                .map(|id| (identity.clone(), entry(id, &format!("content of {id}")))),
        );
        rig
    }

    /// Seed one more entry under `identity` without journaling it.
    pub(crate) async fn append_for_test(&self, identity: &SessionIdentity, id: &str) {
        self.entries
            .lock()
            .unwrap()
            .push((identity.clone(), entry(id, &format!("content of {id}"))));
    }

    pub(crate) fn journal(&self) -> Vec<String> {
        self.journal.lock().unwrap().clone()
    }

    pub(crate) fn clear_journal(&self) {
        self.journal.lock().unwrap().clear();
    }

    /// The ids retained under `identity`, in append order.
    pub(crate) fn ids_of(&self, identity: &SessionIdentity) -> Vec<String> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(owner, _)| owner == identity)
            .map(|(_, e)| e.id.clone())
            .collect()
    }

    fn note(&self, op: &str, identity: &SessionIdentity) {
        self.journal
            .lock()
            .unwrap()
            .push(format!("{op} {}", identity.runtime_key()));
    }

    fn failing(flag: &Mutex<bool>, op: &str) -> Result<(), DomainError> {
        if *flag.lock().unwrap() {
            Err(DomainError::Session(format!("{op} failed")))
        } else {
            Ok(())
        }
    }
}

pub(crate) fn entry(id: &str, content: &str) -> SpillEntry {
    SpillEntry {
        id: id.to_string(),
        tool: "bash".to_string(),
        input_preview: format!("preview of {id}"),
        tokens: 7,
        content: content.to_string(),
    }
}

impl ContextSpillStore for JournalingRetention {
    fn append(&self, identity: &SessionIdentity, entry: &SpillEntry) -> Unit<'_> {
        self.note(&format!("append {}", entry.id), identity);
        let outcome = Self::failing(&self.fail_append, "append");
        if outcome.is_ok() {
            self.entries
                .lock()
                .unwrap()
                .push((identity.clone(), entry.clone()));
        }
        Box::pin(async move { outcome })
    }

    fn recall(&self, identity: &SessionIdentity, id: &SpillId) -> Recall<'_> {
        self.note(&format!("recall {}", id.as_str()), identity);
        let outcome = Self::failing(&self.fail_recall, "recall").map(|()| {
            self.entries
                .lock()
                .unwrap()
                .iter()
                .find(|(owner, e)| owner == identity && e.id == id.as_str())
                .map(|(_, e)| e.clone())
        });
        Box::pin(async move { outcome })
    }

    fn list_entries(&self, identity: &SessionIdentity) -> SpillIndexList<'_> {
        self.note("list", identity);
        let outcome = Self::failing(&self.fail_list, "list").map(|()| {
            Arc::new(
                self.entries
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(owner, _)| owner == identity)
                    .map(|(_, e)| SpillIndex {
                        id: e.id.clone(),
                        tool: e.tool.clone(),
                        input_preview: e.input_preview.clone(),
                        tokens: e.tokens,
                    })
                    .collect::<Vec<_>>(),
            )
        });
        Box::pin(async move { outcome })
    }

    fn has_entries<'a>(&'a self, identity: &'a SessionIdentity) -> SpillPresence<'a> {
        self.note("has_entries", identity);
        let outcome =
            Self::failing(&self.fail_list, "list").map(|()| !self.ids_of(identity).is_empty());
        Box::pin(async move { outcome })
    }

    fn clear(&self, identity: &SessionIdentity) -> Unit<'_> {
        self.note("clear", identity);
        let outcome = Self::failing(&self.fail_clear, "clear");
        if outcome.is_ok() {
            self.entries
                .lock()
                .unwrap()
                .retain(|(owner, _)| owner != identity);
        }
        Box::pin(async move { outcome })
    }

    fn scrub_sync(&self, identity: &SessionIdentity) {
        self.note("scrub", identity);
        self.entries
            .lock()
            .unwrap()
            .retain(|(owner, _)| owner != identity);
    }
}
