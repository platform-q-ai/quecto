//! The bounded environment listing (#2220): which of the registry's
//! records a caller is shown — what it can act on by default, every
//! record on request — in ref-number order, capped, with counts for the
//! rest. Pure selection over a snapshot; presentation is the adapter's.

use crate::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus, ref_number,
};

/// Which environments a bounded listing shows (#2220).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListingScope {
    /// What the caller can act on: every live environment this process
    /// created, the caller's session's earlier ones that are not stopped,
    /// and other sessions' ones a join admits (running, empty, retained).
    /// Stopped ones — the caller's own included — only with
    /// [`ListingScope::All`].
    Actionable,
    /// Every environment on record.
    All,
}

/// One listed environment, whether the calling session created it, and
/// its keep rank: 0 is the row a cap would drop last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedEnvironment {
    pub record: EnvironmentRecord,
    pub own: bool,
    pub keep_rank: usize,
}

/// A bounded listing (#2220): the rows shown, in ref-number order, and
/// counts for what was left out — `hidden` outside the scope, `omitted`
/// inside it but past the limit. `shown + hidden + omitted == total`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentListing {
    pub shown: Vec<ListedEnvironment>,
    pub total: usize,
    pub hidden: usize,
    pub omitted: usize,
    /// Why the inventory may be incomplete — the durable store could not
    /// be read; left for the caller that knows the inventory's health.
    pub diagnostics: Vec<String>,
}

/// How firmly a record belongs in a capped listing (#2220): the lower the
/// tier, the later it is dropped when the cap bites.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    /// Created by this process and not stopped: the caller's live work —
    /// `killing` and `cleanup-failed` ones included, ahead of the
    /// session's earlier running ones: this process owns their settling
    /// (a kill to retry, a teardown under way), which an earlier run's
    /// record, reachable again with `all`, does not need from it.
    CreatedLive,
    /// Restored from an earlier run of the caller's session and not
    /// stopped — joinable, or still something to kill or retry.
    OwnRestoredLive,
    /// Another session's environment a `{"mode":"existing"}` join admits.
    ForeignJoinable,
    /// Created by this process and stopped: listed only with
    /// [`ListingScope::All`], ahead of the rest.
    CreatedStopped,
    /// Everything else: listed only with [`ListingScope::All`].
    Other,
}

fn tier(record: &EnvironmentRecord, own: bool) -> Tier {
    let live = matches!(
        record.status,
        EnvironmentStatus::Running
            | EnvironmentStatus::Retained
            | EnvironmentStatus::Killing
            | EnvironmentStatus::CleanupFailed
    );
    match (record.origin, live, own, record.status.is_joinable()) {
        (EnvironmentOrigin::Created, true, _, _) => Tier::CreatedLive,
        (EnvironmentOrigin::Created, false, _, _) => Tier::CreatedStopped,
        (EnvironmentOrigin::Restored, true, true, _) => Tier::OwnRestoredLive,
        (EnvironmentOrigin::Restored, _, false, true) => Tier::ForeignJoinable,
        (EnvironmentOrigin::Restored, _, _, _) => Tier::Other,
    }
}

/// Select a bounded listing from `records` for the caller `session`: the
/// records in `scope`, at most `limit` of them. When the cap bites the
/// records are kept tier by tier ([`Tier`]) and, within a tier, newest
/// (highest ref) first; the survivors are shown in ref-number order (`C2`
/// before `C10`), with counts for the rest. `diagnostics` is left empty
/// for the caller that knows the inventory's health.
pub fn select_listing(
    records: Vec<EnvironmentRecord>,
    session: &str,
    scope: ListingScope,
    limit: usize,
) -> EnvironmentListing {
    assert!(limit > 0, "a listing shows at least one row");
    let total = records.len();
    let mut in_scope: Vec<(Tier, ListedEnvironment)> = records
        .into_iter()
        .map(|record| {
            let own = is_own(&record, session);
            let tier = tier(&record, own);
            // The keep rank is assigned once the cap has been applied.
            let row = ListedEnvironment {
                record,
                own,
                keep_rank: usize::MAX,
            };
            (tier, row)
        })
        .filter(|(tier, _)| match scope {
            ListingScope::Actionable => {
                matches!(
                    tier,
                    Tier::CreatedLive | Tier::OwnRestoredLive | Tier::ForeignJoinable
                )
            }
            ListingScope::All => true,
        })
        .collect();
    let hidden = total - in_scope.len();
    in_scope.sort_by(|(a_tier, a), (b_tier, b)| {
        a_tier
            .cmp(b_tier)
            .then_with(|| keep_order(&a.record).cmp(&keep_order(&b.record)))
    });
    let omitted = in_scope.len().saturating_sub(limit);
    in_scope.truncate(limit);
    let mut shown: Vec<ListedEnvironment> = in_scope
        .into_iter()
        .enumerate()
        .map(|(keep_rank, (_, row))| ListedEnvironment { keep_rank, ..row })
        .collect();
    shown.sort_by(|a, b| ref_order(&a.record).cmp(&ref_order(&b.record)));
    assert_eq!(
        shown.len() + hidden + omitted,
        total,
        "every record is shown, hidden or omitted exactly once"
    );
    EnvironmentListing {
        shown,
        total,
        hidden,
        omitted,
        diagnostics: Vec::new(),
    }
}

/// Whether the calling session created `record`: this registry committed
/// it, or it was restored from an earlier run of the same named session.
/// A session-less registry owns only what it committed itself.
/// A spawned child journals what it creates under its own session key,
/// which this registry cannot tell apart from an unrelated session's: after
/// a restart such an environment is another session's here (#2220,
/// documented in `docs/container-runtimes.md`).
fn is_own(record: &EnvironmentRecord, session: &str) -> bool {
    match record.origin {
        EnvironmentOrigin::Created => true,
        EnvironmentOrigin::Restored => created_by_session(record, session),
    }
}

/// Whether the named session `session` created `record`, by the creator's
/// key the record carries: a session-less run (empty key) names none.
pub fn created_by_session(record: &EnvironmentRecord, session: &str) -> bool {
    match session {
        "" => false,
        named => record.created_by == named,
    }
}

/// Keep order within a tier: numbered `CN` refs newest (highest) first,
/// anything else last — a ref this registry did not mint is never taken
/// for the newest — by name.
fn keep_order(record: &EnvironmentRecord) -> (std::cmp::Reverse<Option<u64>>, &str) {
    (
        std::cmp::Reverse(ref_number(&record.environment_ref)),
        record.environment_ref.as_str(),
    )
}

/// Ref-number order: numbered `CN` refs by number, anything else after
/// them by name.
fn ref_order(record: &EnvironmentRecord) -> (u64, &str) {
    (
        ref_number(&record.environment_ref).unwrap_or(u64::MAX),
        record.environment_ref.as_str(),
    )
}

#[cfg(test)]
#[path = "environment_listing_tests.rs"]
pub(crate) mod tests;
