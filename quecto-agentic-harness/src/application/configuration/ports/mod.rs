//! Ports of the configuration capability (#1966, #2024): the document
//! store both layers are read from, the writer the one safe write path
//! goes through, the validator the merged or patched document must
//! satisfy, and the trust store that gates the repo-local overlay.
//! Infrastructure implements them; the use cases never touch a filesystem,
//! a `Config` struct or a terminal themselves.

pub mod config_document_store;
pub mod config_document_writer;
pub mod config_validator;
pub mod overlay_trust_store;

pub use config_document_store::ConfigDocumentStore;
pub use config_document_writer::{ConfigDocumentWriter, DocumentLock};
pub use config_validator::ConfigValidator;
pub use overlay_trust_store::{OverlayApproval, OverlayTrust, OverlayTrustStore};
