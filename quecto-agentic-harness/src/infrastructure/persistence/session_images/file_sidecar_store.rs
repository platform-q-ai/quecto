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
use std::collections::{BTreeSet, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use super::sidecar_store::{ImageSidecarStore, SidecarFuture, SidecarRead};
use crate::domain::conversation::stored_images::{
    ImageRef, MAX_STORED_IMAGE_TEXT, VerifiedText, is_sha256_hex,
};
use crate::domain::error::DomainError;
use crate::domain::session_identity::SessionIdentity;
use crate::infrastructure::atomic_write::atomic_write;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;

/// Sidecars are the session's: readable by its owner alone.
const SIDECAR_MODE: u32 = 0o600;

/// How old a temporary file must be before a collection takes it for one an
/// interrupted write left (a write in flight is younger).
const STALE_WRITE: Duration = Duration::from_secs(60);

/// Which file a sidecar is, and its state: a rewrite in place changes the
/// modification or status-change time, a replacement the inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime_ns: i128,
    ctime_ns: i128,
}

impl Stamp {
    fn of(meta: &std::fs::Metadata) -> Self {
        let ns = |secs: i64, nanos: i64| i128::from(secs) * 1_000_000_000 + i128::from(nanos);
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            mtime_ns: ns(meta.mtime(), meta.mtime_nsec()),
            ctime_ns: ns(meta.ctime(), meta.ctime_nsec()),
        }
    }
}

/// Content-addressed image files beside each session's transcript.
#[derive(Debug)]
pub struct FileImageSidecarStore {
    layout: FlatSessionLayout,
    /// The sidecars known to hold their image, as last written or read.
    verified: Mutex<HashMap<PathBuf, Stamp>>,
}

impl FileImageSidecarStore {
    pub fn new(layout: FlatSessionLayout) -> Self {
        Self {
            layout,
            verified: Mutex::default(),
        }
    }

    fn dir(&self, identity: &SessionIdentity) -> PathBuf {
        self.layout.image_dir(identity)
    }

    /// The sidecar named `sha256`; the name must be a digest.
    fn path(&self, identity: &SessionIdentity, sha256: &str) -> PathBuf {
        assert!(is_sha256_hex(sha256), "a sidecar is named by a digest");
        self.dir(identity).join(sha256)
    }

    fn known(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, Stamp>> {
        self.verified.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether the regular file at `path` is still the one last verified.
    fn still_verified(&self, path: &Path, meta: &std::fs::Metadata) -> bool {
        self.known().get(path) == Some(&Stamp::of(meta))
    }

    fn remember(&self, path: &Path, meta: &std::fs::Metadata) {
        self.known().insert(path.to_path_buf(), Stamp::of(meta));
    }

    /// Whether the sidecar at `path` holds exactly `text`, remembered if so.
    async fn holds(&self, path: &Path, text: &str) -> bool {
        match read_sidecar(path).await {
            Ok(Some((held, meta))) if held == text.as_bytes() => {
                self.remember(path, &meta);
                true
            }
            Ok(_) | Err(_) => false,
        }
    }

    async fn write(
        &self,
        identity: &SessionIdentity,
        path: PathBuf,
        text: &str,
    ) -> Result<(), DomainError> {
        let dir = self.dir(identity);
        let bytes = text.as_bytes().to_vec();
        let target = path.clone();
        tokio::task::spawn_blocking(move || {
            create_dir_durably(&dir)?;
            atomic_write(&target, &bytes, Some(SIDECAR_MODE))
        })
        .await
        .map_err(|error| DomainError::Session(format!("image sidecar: write failed: {error}")))?
        .map_err(failed("write"))?;
        match tokio::fs::symlink_metadata(&path).await {
            Ok(meta) => self.remember(&path, &meta),
            Err(_) => {
                self.known().remove(&path);
            }
        }
        Ok(())
    }
}

fn failed(what: &str) -> impl Fn(std::io::Error) -> DomainError + '_ {
    move |error| DomainError::Session(format!("image sidecar: failed to {what}: {error}"))
}

/// Create `dir` and, when it is new, fsync its parent and grandparent so the
/// new entries survive a power cut with the sidecar renamed into them.
fn create_dir_durably(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    for parent in dir.ancestors().skip(1).take(2) {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// The bytes of the regular file at `path` and its metadata: `None` for no
/// file, a link, a FIFO or anything else not a regular file, or a file over
/// the cap. Opened without following a link or blocking.
async fn read_sidecar(path: &Path) -> std::io::Result<Option<(Vec<u8>, std::fs::Metadata)>> {
    use tokio::io::AsyncReadExt;
    let opened = tokio::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .await;
    let file = match opened {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => return Ok(None),
        Err(error) => return Err(error),
    };
    let meta = file.metadata().await?;
    let readable = meta.file_type().is_file() && meta.len() <= MAX_STORED_IMAGE_TEXT as u64;
    if !readable {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_STORED_IMAGE_TEXT as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    match bytes.len() <= MAX_STORED_IMAGE_TEXT {
        true => Ok(Some((bytes, meta))),
        false => Ok(None),
    }
}

impl ImageSidecarStore for FileImageSidecarStore {
    fn put<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        reference: &'a ImageRef,
        text: &'a str,
    ) -> SidecarFuture<'a, ()> {
        Box::pin(async move {
            let path = self.path(identity, &reference.sha256);
            match tokio::fs::symlink_metadata(&path).await {
                Ok(meta) if meta.file_type().is_file() && self.still_verified(&path, &meta) => {
                    return Ok(());
                }
                Ok(meta) if meta.file_type().is_file() && meta.len() == text.len() as u64 => {
                    if self.holds(&path, text).await {
                        return Ok(());
                    }
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(failed("inspect")(error)),
            }
            self.write(identity, path, text).await
        })
    }

    fn get<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        sha256: &'a str,
    ) -> SidecarFuture<'a, SidecarRead> {
        Box::pin(async move {
            if !is_sha256_hex(sha256) {
                return Ok(SidecarRead::Corrupt);
            }
            let path = self.path(identity, sha256);
            let (bytes, meta) = match read_sidecar(&path).await {
                Ok(Some(read)) => read,
                Ok(None) => return Ok(SidecarRead::Corrupt),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(SidecarRead::Missing);
                }
                Err(error) => return Err(failed("read")(error)),
            };
            let Ok(text) = String::from_utf8(bytes) else {
                return Ok(SidecarRead::Corrupt);
            };
            let text = VerifiedText::of(text);
            match text.sha256() == sha256 {
                true => {
                    self.remember(&path, &meta);
                    Ok(SidecarRead::Found(text))
                }
                false => Ok(SidecarRead::Corrupt),
            }
        })
    }

    fn retain_only<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        live: &'a BTreeSet<String>,
    ) -> SidecarFuture<'a, ()> {
        Box::pin(async move {
            let dir = self.dir(identity);
            let garbage = |name: &str, age: Duration| {
                (is_sha256_hex(name) && !live.contains(name))
                    || (is_interrupted_write(name) && age >= STALE_WRITE)
            };
            self.remove_where(&dir, garbage).await?;
            remove_dir_if_empty(&dir).await;
            Ok(())
        })
    }

    fn remove_all<'a>(&'a self, identity: &'a SessionIdentity) -> SidecarFuture<'a, ()> {
        Box::pin(async move {
            let dir = self.dir(identity);
            let ours = |name: &str, _: Duration| is_sha256_hex(name) || is_interrupted_write(name);
            self.remove_where(&dir, ours).await?;
            remove_dir_if_empty(&dir).await;
            Ok(())
        })
    }
}

/// A temporary file an atomic write makes (`.<name>.<uuid>.tmp`).
fn is_interrupted_write(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(".tmp")
}

impl FileImageSidecarStore {
    /// Remove the regular files of `dir` that `chosen` picks by name and age;
    /// no directory is no files.
    async fn remove_where(
        &self,
        dir: &Path,
        chosen: impl Fn(&str, Duration) -> bool,
    ) -> Result<(), DomainError> {
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(failed("list")(error)),
        };
        while let Some(entry) = entries.next_entry().await.map_err(failed("list"))? {
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            let age = meta
                .modified()
                .ok()
                .and_then(|at| SystemTime::now().duration_since(at).ok())
                .unwrap_or_default();
            let name = entry.file_name();
            let picked = name.to_str().is_some_and(|name| chosen(name, age));
            if picked && meta.file_type().is_file() {
                self.known().remove(&entry.path());
                match tokio::fs::remove_file(entry.path()).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(failed("remove")(error)),
                }
            }
        }
        Ok(())
    }
}

/// Remove `dir` once nothing is left in it; a directory still holding a
/// file the store did not pick stays.
async fn remove_dir_if_empty(dir: &Path) {
    let _ = tokio::fs::remove_dir(dir).await;
}

#[cfg(test)]
#[path = "file_sidecar_store_tests.rs"]
mod tests;
