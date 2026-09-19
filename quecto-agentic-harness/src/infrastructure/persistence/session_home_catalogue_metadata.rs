//! The metadata query of the home catalogue (#2010): the store's summary walk
//! joined with the validated home listing. Both halves are stamp-checked
//! against authority on every query and seeded from the derived index, so an
//! unchanged record costs `stat`s, never a transcript read; nothing here
//! opens a transcript at all.
use super::FileSessionHomeCatalogue;
use crate::application::sessions::dto::SessionListQuery;
use crate::application::sessions::ports::SessionStore;
use crate::application::sessions::ports::session_home::{
    SessionHomeCatalogue, SessionMetadataRecord, SessionMetadataSnapshot,
};
use crate::domain::{error::DomainError, session_home::SessionHomeScope};
use std::collections::HashMap;

pub(super) async fn query(
    catalogue: FileSessionHomeCatalogue,
) -> Result<SessionMetadataSnapshot, DomainError> {
    // The walk first: the listing then records each summary it validated.
    let summaries = catalogue.store.list(&SessionListQuery::All).await?;
    let listing = catalogue.list_async().await?;
    let mut diagnostics = listing.diagnostics;
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
