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

use crate::domain::conversation::stored_images::{
    ImageRef, MessageImageRefs, is_storable, restore_images,
};
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
    /// The digests whose sidecar write failed and was warned about.
    failing: Mutex<HashSet<String>>,
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
    /// costs a `stat`, so a save stores them all. Never a save failure: a
    /// sidecar that cannot be written (the directory unreadable, say) is
    /// warned about, the record still names the image, and the image is kept
    /// in memory and stored again on the next save; a load before that keeps
    /// the reference, unloaded.
    pub(in crate::infrastructure::persistence) async fn store(
        &self,
        identity: &SessionIdentity,
        messages: &[Message],
    ) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
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
                    true => {
                        self.put(sidecars.as_ref(), identity, &reference, text)
                            .await
                    }
                    false => self.refuse(&reference, text.len()),
                }
            }
        }
    }

    /// Store one image; a failure only warns (once per image until it is
    /// stored), as the record names the image whatever happens here.
    async fn put(
        &self,
        sidecars: &dyn ImageSidecarStore,
        identity: &SessionIdentity,
        reference: &ImageRef,
        text: &str,
    ) {
        let stored = sidecars.put(identity, reference, text).await;
        let mut failing = self.failing.lock().unwrap_or_else(|e| e.into_inner());
        match stored {
            Ok(()) => {
                failing.remove(&reference.sha256);
            }
            Err(error) => {
                if failing.insert(reference.sha256.clone()) {
                    tracing::warn!(
                        session = identity.runtime_key(),
                        sha256 = reference.sha256,
                        %error,
                        "an image sidecar was not written; the next save tries again"
                    );
                }
            }
        }
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

    /// Put back the images of loaded `messages` from their references, by
    /// message index: each sidecar read is its block again, verbatim; each
    /// one not read stays on its message, unloaded, with a warning.
    pub(in crate::infrastructure::persistence) async fn restore(
        &self,
        identity: &SessionIdentity,
        messages: &mut [Message],
        references: BTreeMap<usize, MessageImageRefs>,
    ) {
        let mut read = BTreeMap::new();
        for (index, refs) in references {
            assert!(
                index < messages.len(),
                "a reference belongs to a message read"
            );
            let tool = self.resolve(identity, refs.tool, &mut read).await;
            let user = self.resolve(identity, refs.user, &mut read).await;
            let unloaded = restore_images(&mut messages[index], tool, user);
            if unloaded > 0 {
                tracing::warn!(
                    session = identity.runtime_key(),
                    index,
                    unloaded,
                    "a saved image could not be read; it is kept, and sent as a marker"
                );
            }
        }
    }

    /// The text of each reference; `read` keeps what was read, so an image
    /// several messages carry is read once.
    async fn resolve(
        &self,
        identity: &SessionIdentity,
        references: Vec<ImageRef>,
        read: &mut BTreeMap<String, Option<String>>,
    ) -> Vec<(ImageRef, Option<String>)> {
        let mut resolved = Vec::with_capacity(references.len());
        for reference in references {
            let text = match read.get(&reference.sha256) {
                Some(text) => text.clone(),
                None => {
                    let text = self.read(identity, &reference.sha256).await;
                    read.insert(reference.sha256.clone(), text.clone());
                    text
                }
            };
            resolved.push((reference, text));
        }
        resolved
    }

    async fn read(&self, identity: &SessionIdentity, sha256: &str) -> Option<String> {
        let sidecars = self.sidecars.as_ref()?;
        let reason = match sidecars.get(identity, sha256).await {
            Ok(SidecarRead::Found(text)) => return Some(text),
            Ok(SidecarRead::Missing) => "missing".to_string(),
            Ok(SidecarRead::Corrupt) => "corrupt (not the image its name is)".to_string(),
            Err(error) => format!("unreadable: {error}"),
        };
        tracing::warn!(session = identity.runtime_key(), sha256, %reason, "image sidecar");
        None
    }

    /// After a save: once a compaction rewrote the transcript, remove the
    /// sidecars it no longer names. Best effort: a failure only warns.
    pub(in crate::infrastructure::persistence) async fn collect(
        &self,
        identity: &SessionIdentity,
        written: Written,
    ) {
        let (Some(sidecars), Written::Compacted(live)) = (self.sidecars.as_ref(), written) else {
            return;
        };
        if let Err(error) = sidecars.retain_only(identity, &live).await {
            tracing::warn!(%error, "image sidecars not collected");
        }
    }

    /// The session's transcript was deleted: so are its sidecars. A failure
    /// only warns; the key's next save collects what is left.
    pub(in crate::infrastructure::persistence) async fn remove_all(
        &self,
        identity: &SessionIdentity,
    ) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        if let Err(error) = sidecars.remove_all(identity).await {
            tracing::warn!(%error, "image sidecars not removed; the next save collects them");
        }
    }
}

/// The messages of a transcript only shown, not resumed (#2424): each image
/// reference stays on its message unloaded, and no sidecar is read.
pub(in crate::infrastructure::persistence) fn leave_unloaded(
    messages: &mut [Message],
    references: BTreeMap<usize, MessageImageRefs>,
) {
    for (index, refs) in references {
        assert!(
            index < messages.len(),
            "a reference belongs to a message read"
        );
        messages[index].unloaded_images.extend(refs.into_unloaded());
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
