//! The one pass both halves of discovery share (#2042). Per record: one
//! stamp; the projection's identity and the walk's summary at that stamp if
//! either half remembers it; else at most ONE read, from which the strict
//! catalogue verdict and the crash-tolerant walk verdict are both drawn; one
//! after-stamp, and only a stable file's verdicts are remembered. The two
//! halves keep their own rules — the walk lists a truncated append the
//! strict catalogue rejects — they just no longer stat and read twice.
//!
//! Locks: the walk's cache is held for the whole scan (as its own walk holds
//! it); the projection and the rejections are taken per record, never the
//! other way round — nothing takes the walk's cache while holding either.
use super::super::session_store::session_store_list::SummaryCache;
use super::super::session_store::session_store_list::session_store_list_scan::session_store_list_record::summary_in;
use super::{FileSessionHomeCatalogue, IndexEntry, Projection, session_home_catalogue_rejections};
use crate::application::sessions::ports::session_home::HomeCatalogueSnapshot;
use crate::domain::{error::DomainError, session::SessionSummary, session_identity::SessionIdentity};
use std::path::Path;

/// What the metadata query takes from the pass besides the listing: the
/// walk's summaries, newest first, and the records it skipped, as
/// `(file name, why)` — the store's own walk word for word.
#[derive(Default)]
pub(super) struct JoinedWalk {
    pub(super) summaries: Vec<SessionSummary>,
    pub(super) skipped: Vec<(String, String)>,
}

/// The metadata query's one pass, off the async runtime: the listing plus
/// the walk's summaries and skips.
pub(super) async fn pass(
    catalogue: FileSessionHomeCatalogue,
) -> Result<
    (
        HomeCatalogueSnapshot,
        Vec<SessionSummary>,
        Vec<(String, String)>,
    ),
    DomainError,
> {
    tokio::task::spawn_blocking(move || {
        let mut walk = JoinedWalk::default();
        let listing = catalogue.list_joined(&mut walk)?;
        Ok((listing, walk.summaries, walk.skipped))
    })
    .await
    .map_err(|e| DomainError::Session(e.to_string()))?
}

impl JoinedWalk {
    pub(super) fn sort(&mut self) {
        self.summaries.sort_by(|a, b| {
            b.updated_unix_secs
                .cmp(&a.updated_unix_secs)
                .then_with(|| a.title.cmp(&b.title))
        });
    }
}

/// Both halves' verdicts on one version of a record.
type Verdicts = (
    Result<SessionIdentity, DomainError>,
    Result<SessionSummary, String>,
);
/// A skip: the identity the listing reports, and the walk's wording.
type Skip = (Result<SessionIdentity, DomainError>, String);

/// One record's verdicts from one pass.
pub(super) struct RecordVerdicts {
    pub(super) identity: Result<SessionIdentity, DomainError>,
    pub(super) summary: Option<SessionSummary>,
}

impl FileSessionHomeCatalogue {
    /// Both halves' verdicts on `path`, from the caches at its stamp or from
    /// one read. The walk's summary or skip lands in `walk`; the strict
    /// identity is returned for the listing.
    pub(super) fn verdicts(
        &self,
        path: &Path,
        cache: &mut SummaryCache,
        walk: &mut JoinedWalk,
    ) -> RecordVerdicts {
        let name = || {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let before = match self.record_stamp(path) {
            Ok(before) => before,
            Err(e) => {
                walk.skipped
                    .push((name(), format!("not a regular session record: {e}")));
                return RecordVerdicts {
                    identity: Err(e),
                    summary: None,
                };
            }
        };
        let identity = self.remembered_identity(path, &before);
        let summary = cache
            .entries
            .get(path)
            .filter(|(at, _)| *at == before)
            .map(|(_, verdict)| verdict.clone());
        let (identity, summary) = match (identity, summary) {
            (Some(identity), Some(summary)) => (identity, summary),
            (identity, summary) => match self.read_both(path, &before, cache, identity, summary) {
                Ok(both) => both,
                Err((identity, why)) => {
                    walk.skipped.push((name(), why));
                    return RecordVerdicts {
                        identity,
                        summary: None,
                    };
                }
            },
        };
        let summary = match summary {
            Ok(summary) => {
                walk.summaries.push(summary.clone());
                Some(summary)
            }
            Err(why) => {
                walk.skipped.push((name(), why));
                None
            }
        };
        RecordVerdicts { identity, summary }
    }

    /// The projection's identity, or the rejection, remembered at `stamp`.
    fn remembered_identity(
        &self,
        path: &Path,
        stamp: &[u64],
    ) -> Option<Result<SessionIdentity, DomainError>> {
        let projected = self
            .projection
            .lock()
            .ok()?
            .get(path)
            .filter(|cached| cached.entry.stamp == stamp)
            .map(|cached| cached.identity.clone());
        if let Some(identity) = projected {
            return Some(Ok(identity));
        }
        self.rejections.lock().ok()?.at(path, stamp).map(Err)
    }

    /// One read serving whichever half has no verdict at `before`. `Err`
    /// is a skip with the walk's wording: an I/O failure (no verdict for
    /// anyone, nothing remembered) or a file that changed around the read.
    fn read_both(
        &self,
        path: &Path,
        before: &[u64],
        cache: &mut SummaryCache,
        identity: Option<Result<SessionIdentity, DomainError>>,
        summary: Option<Result<SessionSummary, String>>,
    ) -> Result<Verdicts, Skip> {
        let unreadable = |e: &std::io::Error| format!("unreadable session file: {e}");
        let bytes = match super::super::session_record_read::read_record(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(path = %path.display(), detail = %e, "skipping unreadable session file while listing sessions");
                let why = unreadable(&e);
                return Err((identity.unwrap_or_else(|| Err(super::error(e))), why));
            }
        };
        self.transcript_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let identity = identity.unwrap_or_else(|| {
            session_home_catalogue_rejections::validated(&bytes, path, &self.layout)
        });
        let summary = summary.unwrap_or_else(|| summary_in(&self.layout, path, &bytes, before));
        drop(bytes);
        // Only a stable file's verdicts are remembered — either half's.
        self.record_stamps
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let after = match session_home_catalogue_rejections::stable(path, before) {
            Ok(after) => after,
            Err(changed) => {
                tracing::warn!(path = %path.display(), "skipping session file changed while listing sessions");
                return Err((
                    Err(changed),
                    "session file changed while listing: retry".into(),
                ));
            }
        };
        cache
            .entries
            .insert(path.to_path_buf(), (after.clone(), summary.clone()));
        match &identity {
            Ok(identity) => {
                let entry = IndexEntry {
                    stamp: after,
                    home: None,
                    summary: None,
                };
                if let Ok(mut projection) = self.projection.lock() {
                    projection.insert(
                        path.to_path_buf(),
                        Projection {
                            identity: identity.clone(),
                            entry,
                        },
                    );
                }
            }
            Err(rejection) => {
                if let Ok(mut rejections) = self.rejections.lock() {
                    rejections.record(path, after, rejection);
                }
            }
        }
        Ok((identity, summary))
    }

    /// A record's stamp, counted (test seam: one per record per warm query).
    fn record_stamp(&self, path: &Path) -> Result<Vec<u64>, DomainError> {
        self.record_stamps
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        super::stamp(path)
    }

    /// How many times the pass stamped a record (test-support).
    #[cfg(feature = "test-support")]
    pub fn record_stamps(&self) -> usize {
        self.record_stamps
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}
