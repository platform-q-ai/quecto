//! The image sidecars of saved sessions (#2424): each image a session's
//! messages carry is stored once beside its transcript, as its exact base64
//! text, named by the SHA-256 of that text; the transcript names it by
//! reference only. The session store writes an image here before any record
//! that names it, reads it back on load, removes what its transcript no
//! longer names once it compacts, and removes everything when the session is
//! deleted. A seam inside persistence, not an application port: only the
//! file session store drives it, and composition wires the adapter
//! ([`super::FileImageSidecarStore`]) into it.
use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;

use crate::domain::conversation::stored_images::ImageRef;
use crate::domain::error::DomainError;
use crate::domain::session_identity::SessionIdentity;

pub type SidecarFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, DomainError>> + Send + 'a>>;

/// What reading one sidecar found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarRead {
    /// The image text whose SHA-256 the name is.
    Found(String),
    /// No sidecar by that name.
    Missing,
    /// Not an image the store wrote: text that does not hash to its name, a
    /// name that is no digest, or a file no image is (a link, a FIFO, too
    /// large, not text).
    Corrupt,
}

/// The content-addressed image sidecars of each session.
pub trait ImageSidecarStore: Send + Sync {
    /// Store `text`, the image `reference` names (its SHA-256 is the
    /// reference's), once and atomically: a sidecar already holding it is
    /// kept, one holding anything else is written again.
    fn put<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        reference: &'a ImageRef,
        text: &'a str,
    ) -> SidecarFuture<'a, ()>;

    /// The sidecar named `sha256`, checked against its name.
    fn get<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        sha256: &'a str,
    ) -> SidecarFuture<'a, SidecarRead>;

    /// Remove every sidecar of `identity` whose digest `live` does not hold.
    fn retain_only<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        live: &'a BTreeSet<String>,
    ) -> SidecarFuture<'a, ()>;

    /// Remove every sidecar of `identity`: the session was deleted.
    fn remove_all<'a>(&'a self, identity: &'a SessionIdentity) -> SidecarFuture<'a, ()>;
}

#[cfg(test)]
#[path = "sidecar_store_contract_tests.rs"]
mod contract_tests;
