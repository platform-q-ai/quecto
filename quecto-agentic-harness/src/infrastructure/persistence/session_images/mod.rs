//! The images a saved session keeps beside its transcript (#2424).
//!
//! The file store never writes an image into a record: each image is a
//! sidecar, its exact base64 text under the SHA-256 of that text, kept by the
//! [`ImageSidecarStore`] seam that composition wires in
//! ([`FileImageSidecarStore`] under the session's `images/` directory); the
//! transcript's records hold [`ImageRefRecord`]s, in each message's order.
//! Around its own writes and reads the file store takes the steps here
//! ([`SessionImages`]):
//! - every image the transcript names is stored before the record naming it
//!   is written; one the store cannot keep (a type no image has, too large)
//!   is still named, with a warning, and is never sent again;
//! - a load restores each reference's text verbatim; one whose sidecar cannot
//!   be read stays on its message as an unloaded image (sent as a marker,
//!   saved again, read again on the next load): never a load failure, and
//!   never a lost image;
//! - a compaction collects the sidecars its transcript no longer names
//!   (session memory's references are information only: a recall never
//!   restores an image);
//! - deleting the session removes its sidecars after its transcript; a
//!   removal that fails leaves sidecars the key's next save collects (its
//!   first write is a compaction).
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, Mutex};

use crate::domain::conversation::stored_images::{ImageRef, MessageImageRefs, is_storable};
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;
pub use sidecar_store::{ImageSidecarStore, SidecarFuture, SidecarRead};

mod file_sidecar_store;
mod sidecar_store;
pub use file_sidecar_store::FileImageSidecarStore;

/// An image reference as a transcript or spill record holds it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(in crate::infrastructure::persistence) struct ImageRefRecord {
    pub(in crate::infrastructure::persistence) sha256: String,
    pub(in crate::infrastructure::persistence) mime_type: String,
}

impl From<&ImageRef> for ImageRefRecord {
    fn from(reference: &ImageRef) -> Self {
        Self {
            sha256: reference.sha256.clone(),
            mime_type: reference.mime_type.clone(),
        }
    }
}

impl From<ImageRefRecord> for ImageRef {
    fn from(record: ImageRefRecord) -> Self {
        Self {
            sha256: record.sha256,
            mime_type: record.mime_type,
        }
    }
}

/// What one save wrote, for the sidecars: a compaction names every image
/// the transcript still holds; an append (or nothing written) removes none.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::infrastructure::persistence) enum Written {
    Compacted(BTreeSet<String>),
    Appended,
}

/// The file store's image steps, over the sidecar port; with no port, images
/// are named by reference only and never stored or read.
#[derive(Default)]
pub(in crate::infrastructure::persistence) struct SessionImages {
    sidecars: Option<Arc<dyn ImageSidecarStore>>,
    /// The digests already warned about as not storable, once each.
    refused: Mutex<HashSet<String>>,
}

impl std::fmt::Debug for SessionImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionImages")
            .field("sidecars", &self.sidecars.is_some())
            .finish_non_exhaustive()
    }
}

impl SessionImages {
    pub(in crate::infrastructure::persistence) fn over(
        sidecars: Arc<dyn ImageSidecarStore>,
    ) -> Self {
        Self {
            sidecars: Some(sidecars),
            ..Self::default()
        }
    }

    /// Store every loaded image `messages` carry, before a record names it.
    /// Each image is hashed once in its life, and a sidecar already verified
    /// costs a `stat`, so a save stores them all.
    pub(in crate::infrastructure::persistence) async fn store(
        &self,
        identity: &SessionIdentity,
        messages: &[Message],
    ) -> Result<(), DomainError> {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return Ok(());
        };
        for message in messages {
            let tool = message
                .image_blocks
                .iter()
                .map(|b| (b.sha256(), b.mime_type, b.data()));
            let user = message
                .user_image_blocks
                .iter()
                .map(|b| (b.sha256(), b.mime_type(), b.data()));
            for (sha256, mime_type, text) in tool.chain(user) {
                let reference = ImageRef {
                    sha256: sha256.to_string(),
                    mime_type: mime_type.to_string(),
                };
                match is_storable(mime_type, text) {
                    true => sidecars.put(identity, &reference, text).await?,
                    false => self.refuse(&reference, text.len()),
                }
            }
        }
        Ok(())
    }

    /// An image the store cannot keep: named in the record, warned once.
    fn refuse(&self, reference: &ImageRef, len: usize) {
        let mut refused = self.refused.lock().unwrap_or_else(|e| e.into_inner());
        if refused.insert(reference.sha256.clone()) {
            tracing::warn!(
                sha256 = reference.sha256,
                mime_type = reference.mime_type,
                len,
                "an image is not stored (a type no image has, or too large); a reload sends a marker"
            );
        }
    }

    /// Put back the images of loaded `messages` from their references: not
    /// yet (#2424), so a reload has none.
    pub(in crate::infrastructure::persistence) async fn restore(
        &self,
        _identity: &SessionIdentity,
        _messages: &mut [Message],
        _references: BTreeMap<usize, MessageImageRefs>,
    ) {
    }

    /// After a save: nothing is collected yet (#2424).
    pub(in crate::infrastructure::persistence) async fn collect(
        &self,
        _identity: &SessionIdentity,
        _written: Written,
    ) {
    }

    /// The session's transcript was deleted: its sidecars are not yet (#2424).
    pub(in crate::infrastructure::persistence) async fn remove_all(
        &self,
        _identity: &SessionIdentity,
    ) {
    }
}

#[cfg(test)]
#[path = "collect_tests.rs"]
mod collect_tests;
#[cfg(test)]
#[path = "faults_tests.rs"]
mod faults_tests;
#[cfg(test)]
#[path = "round_trip_tests.rs"]
mod round_trip_tests;
