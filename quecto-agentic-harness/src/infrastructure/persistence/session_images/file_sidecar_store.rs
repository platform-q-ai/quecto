//! The file-system adapter of the image sidecar port (#2424): one file per
//! image under `<base>/sessions/<sanitized key>/images/`, holding the image's
//! exact base64 text, named by the lowercase hex SHA-256 of that text and
//! written atomically (a temporary file in the same directory, fsynced,
//! renamed into place; a new directory's parents fsynced). Only a name that
//! is a digest is ever read or removed. A sidecar is read without following a
//! link or blocking on a FIFO, only as a regular file of at most
//! [`MAX_STORED_IMAGE_TEXT`] bytes, and only text that hashes to its name is
//! returned. A sidecar this store wrote or read whole is remembered by its
//! file stamp, so saving the same images again costs a `stat` each.
use std::collections::BTreeSet;

use super::sidecar_store::{ImageSidecarStore, SidecarFuture, SidecarRead};
use crate::domain::conversation::stored_images::ImageRef;
use crate::domain::session_identity::SessionIdentity;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;

/// Content-addressed image files beside each session's transcript: not
/// written or read yet (#2424).
#[derive(Debug)]
pub struct FileImageSidecarStore {
    _layout: FlatSessionLayout,
}

impl FileImageSidecarStore {
    pub fn new(layout: FlatSessionLayout) -> Self {
        Self { _layout: layout }
    }
}

impl ImageSidecarStore for FileImageSidecarStore {
    fn put<'a>(
        &'a self,
        _identity: &'a SessionIdentity,
        _reference: &'a ImageRef,
        _text: &'a str,
    ) -> SidecarFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn get<'a>(
        &'a self,
        _identity: &'a SessionIdentity,
        _sha256: &'a str,
    ) -> SidecarFuture<'a, SidecarRead> {
        Box::pin(async { Ok(SidecarRead::Missing) })
    }

    fn retain_only<'a>(
        &'a self,
        _identity: &'a SessionIdentity,
        _live: &'a BTreeSet<String>,
    ) -> SidecarFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn remove_all<'a>(&'a self, _identity: &'a SessionIdentity) -> SidecarFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
#[path = "file_sidecar_store_tests.rs"]
mod tests;
