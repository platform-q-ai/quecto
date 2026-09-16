//! Concrete process launcher for folder-aware resume. A new process is intentionally
//! composed in the target directory so configuration, tools and permissions are reloaded.

use crate::application::sessions::ports::resume_transaction::{
    AtomicResumePersistence, FreshRuntimeLaunch, LaunchCapability, ResumeTransactionResult,
};
use crate::domain::{
    session::Session, session_identity::SessionIdentity, session_scope::SessionScopeMetadata,
};
use crate::infrastructure::persistence::{
    session_layout::FlatSessionLayout, session_ownership::SessionOwnershipRegistry,
};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

#[derive(Debug)]
pub struct FileSessionScopeTransaction {
    layout: FlatSessionLayout,
    ownership: SessionOwnershipRegistry,
}
impl FileSessionScopeTransaction {
    pub fn new(layout: FlatSessionLayout) -> Self {
        Self {
            layout,
            ownership: SessionOwnershipRegistry::default(),
        }
    }
    fn intent_path(&self, key: &SessionIdentity) -> PathBuf {
        self.layout
            .session_file(key)
            .with_extension("resume-intent")
    }
    /// Deterministic crash recovery: a completed rename is authoritative; otherwise
    /// discard the incomplete stage. Catalogue repair happens independently.
    pub fn recover(&self, key: &SessionIdentity) -> Result<(), String> {
        let intent = self.intent_path(key);
        if !intent.exists() {
            return Ok(());
        }
        let target = self.layout.session_file(key);
        let stage = target.with_extension("tmp");
        if stage.exists() {
            std::fs::rename(&stage, &target).map_err(|e| format!("recover staged session: {e}"))?;
        }
        std::fs::remove_file(intent).map_err(|e| format!("clear resume intent: {e}"))
    }
}
impl AtomicResumePersistence for FileSessionScopeTransaction {
    fn claim(&self, key: &SessionIdentity) -> Result<(), String> {
        self.ownership
            .claim(&self.layout, key)
            .map_err(|e| e.to_string())
    }
    fn release(&self, key: &SessionIdentity) {
        self.ownership.release(key)
    }
    fn commit<'a>(
        &'a self,
        session: &'a Session,
        scope: &'a SessionScopeMetadata,
    ) -> ResumeTransactionResult<'a> {
        Box::pin(async move {
            self.claim(&session.key)?;
            tokio::fs::create_dir_all(self.layout.sessions_dir())
                .await
                .map_err(|e| format!("create session directory: {e}"))?;
            let intent = self.intent_path(&session.key);
            tokio::fs::write(&intent, b"resume-v1\n")
                .await
                .map_err(|e| format!("write resume intent: {e}"))?;
            let result =
                crate::infrastructure::persistence::session_store::write_compacted_with_scope(
                    &self.layout.session_file(&session.key),
                    session,
                    Some(scope),
                )
                .await
                .map_err(|e| e.to_string());
            match result {
                Ok(()) => tokio::fs::remove_file(intent)
                    .await
                    .map_err(|e| format!("clear resume intent: {e}")),
                Err(error) => {
                    self.rollback(&session.key);
                    Err(error)
                }
            }
        })
    }
    fn rollback(&self, key: &SessionIdentity) {
        let tmp = self.layout.session_file(key).with_extension("tmp");
        let _ = std::fs::remove_file(tmp);
        let _ = std::fs::remove_file(self.intent_path(key));
    }
}

#[derive(Debug)]
pub struct FreshRuntimeLauncher {
    executable: PathBuf,
}
impl FreshRuntimeLauncher {
    pub fn current_executable() -> Result<Self, String> {
        std::env::current_exe()
            .map(|executable| Self { executable })
            .map_err(|e| format!("cannot locate current executable: {e}"))
    }
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }
    pub fn launch(&self, target: &Path, session_key: &str) -> Result<Child, String> {
        if !(target.is_absolute() && target.is_dir()) {
            return Err("target folder is not an available absolute directory".into());
        }
        if session_key.is_empty() {
            return Err("session key must not be empty".into());
        }
        Command::new(&self.executable)
            .current_dir(target)
            .arg("--session")
            .arg(session_key)
            .spawn()
            .map_err(|e| format!("failed to launch fresh runtime: {e}"))
    }
}
impl FreshRuntimeLaunch for FreshRuntimeLauncher {
    fn capability(&self, target: &str) -> LaunchCapability {
        match std::fs::canonicalize(target) {
            Ok(path) if path.is_dir() => LaunchCapability::Ready {
                canonical_directory: path.to_string_lossy().into_owned(),
            },
            Ok(_) => LaunchCapability::Unavailable {
                reason: "target is not a directory".into(),
            },
            Err(error) => LaunchCapability::Unavailable {
                reason: format!("target unavailable: {error}"),
            },
        }
    }
    fn launch<'a>(
        &'a self,
        directory: &'a str,
        key: &'a SessionIdentity,
    ) -> ResumeTransactionResult<'a> {
        Box::pin(async move {
            self.launch(Path::new(directory), key.runtime_key())
                .map(|_| ())
        })
    }
}

#[cfg(test)]
#[path = "resume_decision_adapter_tests.rs"]
mod tests;
