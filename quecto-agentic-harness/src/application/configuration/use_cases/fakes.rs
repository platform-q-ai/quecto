//! In-memory doubles of the configuration ports for use-case tests.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::application::configuration::ports::{
    ConfigDocumentStore, ConfigDocumentWriter, ConfigValidator, OverlayApproval, OverlayTrust,
    OverlayTrustStore,
};

/// Files by path; `broken` paths fail to read (a present, unreadable entry).
#[derive(Default)]
pub struct MemoryStore {
    pub files: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
    pub broken: BTreeSet<PathBuf>,
    pub fail_writes: bool,
}

impl MemoryStore {
    pub fn with(files: &[(&str, &str)]) -> Arc<Self> {
        let store = Self::default();
        for (path, content) in files {
            store
                .files
                .lock()
                .unwrap()
                .insert(PathBuf::from(path), content.as_bytes().to_vec());
        }
        Arc::new(store)
    }

    pub fn content(&self, path: &str) -> Option<String> {
        self.files
            .lock()
            .unwrap()
            .get(Path::new(path))
            .map(|bytes| String::from_utf8(bytes.clone()).unwrap())
    }
}

impl ConfigDocumentStore for MemoryStore {
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, String> {
        if self.broken.contains(path) {
            return Err("permission denied".into());
        }
        Ok(self.files.lock().unwrap().get(path).cloned())
    }

    fn is_present(&self, path: &Path) -> bool {
        self.broken.contains(path) || self.files.lock().unwrap().contains_key(path)
    }
}

/// Writes compact JSON plus a newline into the same in-memory files.
impl ConfigDocumentWriter for MemoryStore {
    fn write(&self, path: &Path, document: &Value) -> Result<(), String> {
        if self.fail_writes {
            return Err("disk full".into());
        }
        let mut bytes = serde_json::to_vec(document).unwrap();
        bytes.push(b'\n');
        self.files.lock().unwrap().insert(path.to_path_buf(), bytes);
        Ok(())
    }
}

/// A validator that rejects a document whose `agents.defaults.effort` is
/// `"bogus"` and a `container_scripts` key at resolve time, and rewrites
/// `workflow.marker` to the document's own path so resolution is visible.
#[derive(Default)]
pub struct FakeValidator {
    pub validated: Mutex<Vec<Value>>,
}

impl ConfigValidator for FakeValidator {
    fn resolve(&self, mut document: Value, path: &Path) -> Result<Value, String> {
        if document.get("container_scripts").is_some() {
            return Err("the `container_scripts` key was renamed".into());
        }
        if let Some(workflow) = document.get_mut("workflow").and_then(Value::as_object_mut)
            && workflow.contains_key("marker")
        {
            workflow.insert("marker".into(), Value::String(path.display().to_string()));
        }
        Ok(document)
    }

    fn validate(&self, document: &Value) -> Result<(), String> {
        self.validated.lock().unwrap().push(document.clone());
        if document.pointer("/agents/defaults/effort") == Some(&Value::String("bogus".into())) {
            return Err("invalid effort level 'bogus'".into());
        }
        Ok(())
    }
}

/// Trust by exact (path, content) pairs; approvals are recorded.
#[derive(Default)]
pub struct FakeTrust {
    pub approved: Mutex<BTreeSet<(PathBuf, Vec<u8>)>>,
    pub fail_approve: bool,
}

impl FakeTrust {
    pub fn trusting(path: &str, content: &str) -> Arc<Self> {
        let trust = Self::default();
        trust
            .approved
            .lock()
            .unwrap()
            .insert((PathBuf::from(path), content.as_bytes().to_vec()));
        Arc::new(trust)
    }

    pub fn is_approved(&self, path: &str, content: &[u8]) -> bool {
        self.approved
            .lock()
            .unwrap()
            .contains(&(PathBuf::from(path), content.to_vec()))
    }
}

impl OverlayTrustStore for FakeTrust {
    fn decide(&self, path: &Path, content: &[u8]) -> OverlayTrust {
        if self.is_approved(&path.display().to_string(), content) {
            OverlayTrust::Trusted
        } else {
            OverlayTrust::Untrusted {
                fingerprint: format!("fp-{}", content.len()),
            }
        }
    }

    fn approve(&self, path: &Path, content: &[u8]) -> Result<OverlayApproval, String> {
        if self.fail_approve {
            return Err("store unwritable".into());
        }
        self.approved
            .lock()
            .unwrap()
            .insert((path.to_path_buf(), content.to_vec()));
        Ok(OverlayApproval {
            path: path.to_path_buf(),
            fingerprint: format!("fp-{}", content.len()),
        })
    }
}
