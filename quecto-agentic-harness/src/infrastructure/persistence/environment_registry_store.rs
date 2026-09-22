//! The durable environment registry of one base directory (#2024 S4d):
//! `<base_dir>/environments.json`, one document holding the ref counter
//! and every record keyed by ref, written atomically (tmp + fsync + rename
//! through `atomic_write`, 0600) and — because a rename keeps a file whole
//! but not an *update* — every read → change → write cycle under the same
//! exclusive `flock` the configuration writer uses (`<base_dir>/locks/…`),
//! so two sessions allocating a ref or recording at once serialise
//! instead of overwriting each other. The environments capability's
//! [`EnvironmentRegistryStore`] port.
//!
//! Members are not stored: they belong to the session that launched them.
//! A conditional write (`correct`) merges its metadata over the file's;
//! an unconditional one (`record`) replaces the record whole.
//! An unreadable document (corrupt JSON, a newer version) is an error the
//! restore reports; it is never silently replaced — a write on top of it
//! fails the same way, so a broken file stops the registry rather than
//! losing what it held.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::environments::dto::CorrectionOutcome;
use crate::application::environments::ports::EnvironmentRegistryStore;
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus, merge_metadata, ref_number,
};
use crate::infrastructure::atomic_write::atomic_write;
use crate::infrastructure::config::writer::{exclusive_hold, lock_dir_for};

/// The document's name under the base directory.
pub const REGISTRY_FILE_NAME: &str = "environments.json";

const DOCUMENT_VERSION: u32 = 1;
const FILE_MODE: u32 = 0o600;

#[derive(Clone)]
pub struct FileEnvironmentRegistryStore {
    path: PathBuf,
    lock_dir: PathBuf,
    /// Seconds since the Unix epoch, for the age of an in-flight mint.
    now: Arc<dyn Fn() -> u64 + Send + Sync>,
}

/// How long a minted ref nobody recorded keeps its number (#2070): a
/// create that failed after minting, or a harness that died mid-create,
/// leaves a mint nobody settles; past this it no longer blocks reuse. A
/// create is unbounded (its clone can be slow), so this is generous — an
/// hour — and `record` refuses to overwrite another environment under the
/// same ref should a create outlive even that.
pub const PENDING_REF_GRACE_SECS: u64 = 60 * 60;

impl std::fmt::Debug for FileEnvironmentRegistryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileEnvironmentRegistryStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl FileEnvironmentRegistryStore {
    pub fn for_base_dir(base_dir: &Path) -> Self {
        Self::for_base_dir_at(base_dir, || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0)
        })
    }

    /// The store with its own clock (tests age an in-flight mint).
    pub fn for_base_dir_at(base_dir: &Path, now: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self {
            path: base_dir.join(REGISTRY_FILE_NAME),
            lock_dir: lock_dir_for(base_dir),
            now: Arc::new(now),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<Document, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                let document: Document = serde_json::from_slice(&bytes).map_err(|error| {
                    format!(
                        "{} is not a valid environment registry: {error}",
                        self.path.display()
                    )
                })?;
                if document.version != DOCUMENT_VERSION {
                    return Err(format!(
                        "{} is environment registry version {}, this harness reads version {DOCUMENT_VERSION}",
                        self.path.display(),
                        document.version
                    ));
                }
                Ok(document)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Document::default()),
            Err(error) => Err(format!("{}: {error}", self.path.display())),
        }
    }

    fn write(&self, document: &Document) -> Result<(), String> {
        let mut bytes = serde_json::to_vec_pretty(document)
            .map_err(|error| format!("environment registry could not be encoded: {error}"))?;
        bytes.push(b'\n');
        atomic_write(&self.path, &bytes, Some(FILE_MODE))
            .map_err(|error| format!("{}: {error}", self.path.display()))
    }

    /// Read, change, write under the exclusive hold.
    fn update<T>(&self, change: impl FnOnce(&mut Document) -> T) -> Result<T, String> {
        let _hold = exclusive_hold(&self.lock_dir, &self.path)?;
        let mut document = self.read()?;
        let outcome = change(&mut document);
        self.write(&document)?;
        Ok(outcome)
    }
}

impl EnvironmentRegistryStore for FileEnvironmentRegistryStore {
    /// The next ref is one above every number that still exists (#2070): a
    /// recorded environment, whatever its status, or a mint younger than a
    /// create can take that nobody has recorded yet — never the old
    /// monotonic counter, so once everything is collected the next
    /// container is `C1` again, and a concurrent session's in-flight create
    /// keeps its number until it records or gives up.
    fn allocate_ref(&self) -> Result<u64, String> {
        let now = (self.now)();
        self.update(|document| {
            document
                .pending_refs
                .retain(|_, minted_at| now.saturating_sub(*minted_at) <= PENDING_REF_GRACE_SECS);
            let recorded = document
                .environments
                .keys()
                .filter_map(|key| ref_number(key));
            let in_flight = document.pending_refs.keys().copied();
            let next = recorded.chain(in_flight).max().unwrap_or(0) + 1;
            document.pending_refs.insert(next, now);
            // Kept for older readers of the document; no longer the rule.
            document.next_ref = next;
            next
        })
    }

    fn release_ref(&self, number: u64) -> Result<(), String> {
        self.update(|document| {
            document.pending_refs.remove(&number);
        })
    }

    fn load(&self) -> Result<Vec<EnvironmentRecord>, String> {
        let _hold = exclusive_hold(&self.lock_dir, &self.path)?;
        let document = self.read()?;
        let mut records: Vec<EnvironmentRecord> = document
            .environments
            .into_iter()
            .map(|(environment_ref, wire)| wire.into_record(environment_ref))
            .collect();
        records.sort_by_key(|record| ref_number(&record.environment_ref).unwrap_or(u64::MAX));
        Ok(records)
    }

    /// A ref names one environment: a record already on file under it for
    /// a different environment is never replaced (#2070 — a create that
    /// outlived its mint's grace while another session took the number).
    fn record(&self, record: &EnvironmentRecord) -> Result<(), String> {
        self.update(|document| {
            if let Some(other) = document
                .environments
                .get(&record.environment_ref)
                .filter(|on_file| on_file.environment_uuid != record.environment_uuid)
            {
                return Err(format!(
                    "ref {} already records environment {} ({}); refusing to overwrite it with {}",
                    record.environment_ref,
                    other.environment_uuid,
                    other.environment_id,
                    record.environment_id
                ));
            }
            if let Some(number) = ref_number(&record.environment_ref) {
                document.next_ref = document.next_ref.max(number);
                // The mint is settled: the record keeps the number now.
                document.pending_refs.remove(&number);
            }
            document
                .environments
                .insert(record.environment_ref.clone(), RecordWire::from(record));
            Ok(())
        })?
    }

    fn correct(
        &self,
        record: &EnvironmentRecord,
        expected: &EnvironmentStatus,
    ) -> Result<CorrectionOutcome, String> {
        let _hold = exclusive_hold(&self.lock_dir, &self.path)?;
        let mut document = self.read()?;
        let Some(current) = document.environments.get(&record.environment_ref) else {
            return Ok(CorrectionOutcome::Forgotten);
        };
        if EnvironmentStatus::from(current.status) != *expected {
            let current = document
                .environments
                .remove(&record.environment_ref)
                .expect("looked up above");
            return Ok(CorrectionOutcome::Superseded(Box::new(
                current.into_record(record.environment_ref.clone()),
            )));
        }
        // A conditional write is another session's view of the record:
        // its metadata goes over the file's, key by key (round 3, #2033),
        // so a key it never saw — the creator's retention reason — is
        // kept, and one it names is its value.
        let mut wire = RecordWire::from(record);
        wire.metadata = merge_metadata(current.metadata.clone(), &wire.metadata);
        document
            .environments
            .insert(record.environment_ref.clone(), wire);
        self.write(&document)?;
        Ok(CorrectionOutcome::Applied)
    }

    fn forget(&self, environment_ref: &str) -> Result<(), String> {
        self.update(|document| {
            document.environments.remove(environment_ref);
        })
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Document {
    #[serde(default = "current_version")]
    version: u32,
    #[serde(default)]
    next_ref: u64,
    /// Minted refs not yet recorded, by number, with the second they were
    /// minted (#2070). Absent in older documents.
    #[serde(default)]
    pending_refs: BTreeMap<u64, u64>,
    #[serde(default)]
    environments: BTreeMap<String, RecordWire>,
}

fn current_version() -> u32 {
    DOCUMENT_VERSION
}

impl Default for Document {
    fn default() -> Self {
        Self {
            version: DOCUMENT_VERSION,
            next_ref: 0,
            pending_refs: BTreeMap::new(),
            environments: BTreeMap::new(),
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct RecordWire {
    environment_id: String,
    environment_uuid: String,
    #[serde(default)]
    name: Option<String>,
    workspace_path: PathBuf,
    #[serde(default)]
    repository: String,
    #[serde(default)]
    config: String,
    #[serde(default)]
    exec: Vec<String>,
    #[serde(default)]
    kill: Vec<String>,
    #[serde(default)]
    cleanup: Vec<String>,
    #[serde(default)]
    inspect: Vec<String>,
    status: StatusWire,
    #[serde(default)]
    metadata: serde_json::Value,
    #[serde(default)]
    last_error: Option<String>,
    #[serde(default)]
    created_by: String,
    #[serde(default)]
    created_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StatusWire {
    Running,
    Killing,
    Stopped,
    CleanupFailed,
    Retained,
}

impl From<&EnvironmentStatus> for StatusWire {
    fn from(status: &EnvironmentStatus) -> Self {
        match status {
            EnvironmentStatus::Running => Self::Running,
            EnvironmentStatus::Killing => Self::Killing,
            EnvironmentStatus::Stopped => Self::Stopped,
            EnvironmentStatus::CleanupFailed => Self::CleanupFailed,
            EnvironmentStatus::Retained => Self::Retained,
        }
    }
}

impl From<StatusWire> for EnvironmentStatus {
    fn from(status: StatusWire) -> Self {
        match status {
            StatusWire::Running => Self::Running,
            StatusWire::Killing => Self::Killing,
            StatusWire::Stopped => Self::Stopped,
            StatusWire::CleanupFailed => Self::CleanupFailed,
            StatusWire::Retained => Self::Retained,
        }
    }
}

impl From<&EnvironmentRecord> for RecordWire {
    fn from(record: &EnvironmentRecord) -> Self {
        Self {
            environment_id: record.environment_id.clone(),
            environment_uuid: record.environment_uuid.clone(),
            name: record.name.clone(),
            workspace_path: record.workspace_path.clone(),
            repository: record.repository.clone(),
            config: record.script_name.clone(),
            exec: record.retained_exec_argv.clone(),
            kill: record.retained_kill_argv.clone(),
            cleanup: record.retained_cleanup_argv.clone(),
            inspect: record.retained_inspect_argv.clone(),
            status: StatusWire::from(&record.status),
            metadata: record.metadata.clone(),
            last_error: record.last_error.clone(),
            created_by: record.created_by.clone(),
            created_at: record.created_at,
        }
    }
}

impl RecordWire {
    fn into_record(self, environment_ref: String) -> EnvironmentRecord {
        EnvironmentRecord {
            environment_ref,
            environment_id: self.environment_id,
            environment_uuid: self.environment_uuid,
            name: self.name,
            workspace_path: self.workspace_path,
            repository: self.repository,
            script_name: self.config,
            retained_exec_argv: self.exec,
            retained_kill_argv: self.kill,
            retained_cleanup_argv: self.cleanup,
            retained_inspect_argv: self.inspect,
            members: Vec::new(),
            status: self.status.into(),
            metadata: self.metadata,
            last_error: self.last_error,
            // Whoever loads it did not create it here; the restore marks
            // the origin, the store only carries the creator's key.
            origin: EnvironmentOrigin::Restored,
            created_by: self.created_by,
            created_at: self.created_at,
        }
    }
}

#[cfg(test)]
#[path = "environment_registry_store_tests.rs"]
mod tests;
