//! The coordination board's `members` rows.

use crate::application::swarm::dto::{
    LaunchIdentity, MemberClaimCounts, MemberRow, MemberStatusRow, NewMember,
};
use crate::domain::swarm::entities::records::MemberRecord;
use crate::domain::swarm::value_objects::board_error::BoardError;
use serde_json::Value;

/// The `members` rows.
pub trait BoardMembers {
    fn member(&self, id: &str) -> Result<Option<MemberRecord>, BoardError>;
    /// The operation gate authorised this transaction's caller as a member
    /// of the run with the op's own access (#2313 review M2). A metered
    /// call notes it, so a refusal the op then makes still records the
    /// caller's role; it runs no statement and changes nothing.
    fn caller_authorized(&self);
    /// Every member row, in store order.
    fn members(&self) -> Result<Vec<MemberRow>, BoardError>;
    /// Members whose status is `live` or `reserved`.
    fn usage(&self) -> Result<i64, BoardError>;
    /// Members whose status is not `dead` (SQL `status!='dead'`: a NULL
    /// status is not counted), as `create` counts them.
    fn not_dead(&self) -> Result<i64, BoardError>;
    /// `member_claim_counts(coordinator)` (#1969).
    fn claim_counts(&self, coordinator: Option<&str>) -> Result<MemberClaimCounts, BoardError>;
    fn insert_member(&self, member: &NewMember) -> Result<(), BoardError>;
    /// `SELECT * FROM members WHERE id=?` (and `AND reservation=?` when
    /// `reservation` is given) as `dict(row)`. The id and reservation are
    /// the caller's values, bound as Python's `sqlite3` binds them (so `5`
    /// finds the id `'5'` through TEXT affinity, and a NULL matches
    /// nothing); a value it cannot bind is refused naming its parameter.
    fn member_row(
        &self,
        id: &Value,
        reservation: Option<&Value>,
    ) -> Result<Option<MemberRow>, BoardError>;
    /// `SELECT status FROM members WHERE id=?` (#2275): only the status,
    /// so no other column of the row is read or refused. The id is bound
    /// as [`BoardMembers::member_row`] binds it; `None` when no row
    /// matches.
    fn member_status(&self, id: &Value) -> Result<Option<MemberStatusRow>, BoardError>;
    /// `Transaction.reserve_member`: a `reserved` row with no process yet,
    /// launched by `launcher` (#1961), the reservation bound as given.
    fn reserve_member(
        &self,
        id: &str,
        reservation: &Value,
        launcher: &str,
    ) -> Result<(), BoardError>;
    /// The member is live in the process `launch` at `socket`, each bound
    /// as given.
    fn activate_member(
        &self,
        id: &Value,
        launch: &LaunchIdentity,
        socket: &Value,
    ) -> Result<(), BoardError>;
    /// The launcher's record of the member's process, bound as given.
    fn record_launch(&self, id: &Value, launch: &LaunchIdentity) -> Result<(), BoardError>;
    /// `UPDATE members SET status='dead' WHERE id=?`: an admission that
    /// was never launched is abandoned, or a death is confirmed (#2277).
    fn mark_member_dead(&self, id: &Value) -> Result<(), BoardError>;
    /// `SELECT launcher FROM members WHERE id=?` (#1961, #2277): `None`
    /// when no row matches, and `Some(None)` for a launcher that is NULL
    /// or not text. The id is bound as [`BoardMembers::member_row`] binds
    /// it.
    fn member_launcher(&self, id: &Value) -> Result<Option<Option<String>>, BoardError>;
    /// `lost_after_activation(member)` (#1961, #2277): whether `member`'s
    /// latest `scope_unknown` event is newer than its latest `activated`
    /// one, in the one ordered scan of those two kinds
    /// [`BoardMembers::lost_members`] runs. An event names `member` when
    /// its detail's `member` equals it by Python's `==`.
    fn lost_after_activation(&self, member: &Value) -> Result<bool, BoardError>;
    /// `SELECT id,status FROM members WHERE id IN (…)` (#1969, #2277): the
    /// status of each of `ids` that has a row (NULL, or a value that is not
    /// text, is `None`), in store order.
    fn member_statuses(&self, ids: &[&str]) -> Result<Vec<(String, Option<String>)>, BoardError>;
    /// The member's endpoint, bound as given; a member without a row is
    /// left as it is.
    fn set_socket(&self, id: &str, socket: &Value) -> Result<(), BoardError>;
    /// `swarm_repository.lost_members` (#1924, #1961, #1969): those of
    /// `members` whose latest `scope_unknown` event is newer than their
    /// latest `activated` one, in one ordered scan of those two event
    /// kinds, in the order given. An event names its member by the text
    /// of its detail's `member`.
    fn lost_members(&self, members: &[&str]) -> Result<Vec<String>, BoardError>;
}
