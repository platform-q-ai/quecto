//! Ports of the configuration capability (#1966, #2024): the document
//! store both layers are read from, the writer the one safe write path
//! goes through, the validator the merged or patched document must
//! satisfy, the trust store that gates the repo-local overlay, and the
//! locator that names a linked worktree's main worktree; for `quecto
//! status` (#1843), the run settings a document yields and the stored
//! credentials the auth capability reports.
//! Infrastructure implements them; the use cases never touch a filesystem,
//! a `Config` struct or a terminal themselves.

pub mod config_document_store;
pub mod config_document_writer;
pub mod config_validator;
pub mod main_worktree_locator;
pub mod overlay_trust_store;
pub mod run_settings;
pub mod stored_credentials;

pub use config_document_store::{ConfigDocumentStore, OverlayDocument};
pub use config_document_writer::{ConfigDocumentWriter, DocumentLock};
pub use config_validator::ConfigValidator;
pub use main_worktree_locator::{MainWorktreeLocator, WorktreeLink};
pub use overlay_trust_store::{OverlayApproval, OverlayTrust, OverlayTrustStore};
pub use run_settings::{RunSettings, RunSettingsSource};
pub use stored_credentials::{
    StoredCredential, StoredCredentialPresence, StoredCredentialsUnavailable,
};
