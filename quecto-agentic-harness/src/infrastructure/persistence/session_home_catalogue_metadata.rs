//! The metadata query of the home catalogue (#2010): the store's summary walk
//! joined with the validated home listing — from ONE pass over the directory
//! (#2042): each record is stamped once per query and read at most once, the
//! walk's summary and the strict identity drawn from the same bytes. Nothing
//! is ever read to MATCH: matching sees listing titles only.
use super::FileSessionHomeCatalogue;
use crate::application::sessions::ports::session_home::{
    SessionHomeCatalogue, SessionMetadataRecord, SessionMetadataSnapshot,
};
use crate::domain::{error::DomainError, session_home::SessionHomeScope};
use std::collections::HashMap;

pub(super) async fn query(
    catalogue: FileSessionHomeCatalogue,
) -> Result<SessionMetadataSnapshot, DomainError> {
    // One pass (#2042): listing, walk summaries and skips from the same stamps and bytes.
    let (listing, summaries, skipped) =
        super::session_home_catalogue_joined::pass(catalogue.clone()).await?;
    let mut diagnostics = listing.diagnostics;
    super::session_home_catalogue_rejections::name_skipped(&mut diagnostics, skipped);
    let mut homes: HashMap<String, SessionHomeScope> = listing
        .entries
        .into_iter()
        .map(|(identity, home)| (identity.runtime_key().to_string(), home))
        .collect();
    let records = summaries
        .into_iter()
        .map(|summary| {
            // A store-listed record the strict catalogue rejected (an append
            // cut short by a crash) keeps the home admission would read.
            let home = homes.remove(&summary.key).unwrap_or_else(|| {
                catalogue.read(&summary.identity).unwrap_or_else(|error| {
                    diagnostics.push(format!("record not in catalogue; home unreadable: {error}"));
                    SessionHomeScope::Unavailable("record not in catalogue".into())
                })
            });
            SessionMetadataRecord { summary, home }
        })
        .collect();
    Ok(SessionMetadataSnapshot {
        records,
        diagnostics,
        rebuilt: listing.rebuilt,
    })
}
