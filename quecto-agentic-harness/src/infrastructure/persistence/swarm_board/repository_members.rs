//! `BoardMembers` over the SQLite store (#2270, #2271): the `members` rows
//! by Python's SQL (`swarm.py`, `swarm_repository.py`), split out of
//! `repository.rs` to keep each file within its size budget. The
//! membership statements (#2271 round-1 review M1) bind the caller's
//! member, reservation, pid, start time and socket as Python's `sqlite3`
//! binds them ([`loose`]), so the column affinity stores and compares
//! them as Python's board does.
use rusqlite::OptionalExtension;
use rusqlite::types::Value as SqlValue;
use serde_json::Value;

use super::binding;
use super::repository::{
    ACTIVE_CLAIM, PYTHON_PID_PARAMETER, PYTHON_SOCKET_PARAMETER, PYTHON_STARTED_PARAMETER,
    SqliteBoard, failed, fetched, loose, member_row as row_dict,
};
use crate::application::swarm::dto::{LaunchIdentity, MemberClaimCounts, MemberRow, NewMember};
use crate::application::swarm::ports::BoardMembers;
use crate::domain::swarm::{BoardError, MemberRecord, MemberState};

impl BoardMembers for SqliteBoard<'_> {
    fn member(&self, id: &str) -> Result<Option<MemberRecord>, BoardError> {
        self.connection
            .query_row("SELECT * FROM members WHERE id=?", [id], |row| {
                fetched(row)?;
                Ok(MemberRecord {
                    id: row.get("id")?,
                    status: row
                        .get::<_, Option<String>>("status")?
                        .map(MemberState::new),
                    reservation: row.get("reservation")?,
                })
            })
            .optional()
            .map_err(failed)
    }

    fn members(&self) -> Result<Vec<MemberRow>, BoardError> {
        let mut statement = self
            .connection
            .prepare("SELECT * FROM members")
            .map_err(failed)?;
        let rows = statement.query_map([], row_dict).map_err(failed)?;
        rows.collect::<rusqlite::Result<_>>().map_err(failed)
    }

    fn usage(&self) -> Result<i64, BoardError> {
        self.count("SELECT count(*) FROM members WHERE status IN ('live','reserved')")
    }

    fn not_dead(&self) -> Result<i64, BoardError> {
        self.count("SELECT count(*) FROM members WHERE status!='dead'")
    }

    fn claim_counts(&self, coordinator: Option<&str>) -> Result<MemberClaimCounts, BoardError> {
        let members_without_claim = self
            .connection
            .query_row(
                &format!(
                    "SELECT count(*) FROM members m WHERE m.status IN ('live','reserved') AND m.id IS NOT ? \
                     AND NOT EXISTS (SELECT 1 FROM tasks t WHERE t.owner=m.id AND t.status IN {ACTIVE_CLAIM})"
                ),
                [coordinator],
                |row| row.get(0),
            )
            .map_err(failed)?;
        let members_dead = self.count("SELECT count(*) FROM members WHERE status='dead'")?;
        Ok(MemberClaimCounts {
            members_without_claim,
            members_dead,
        })
    }

    fn insert_member(&self, member: &NewMember) -> Result<(), BoardError> {
        let parameters = [
            SqlValue::Text(member.id.clone()),
            SqlValue::Text(member.reservation.clone()),
            SqlValue::Text(member.status.as_str().to_owned()),
            loose(PYTHON_PID_PARAMETER, &member.pid)?,
            loose(PYTHON_STARTED_PARAMETER, &member.started)?,
            loose(PYTHON_SOCKET_PARAMETER, &member.socket)?,
            member
                .launcher
                .clone()
                .map_or(SqlValue::Null, SqlValue::Text),
        ];
        binding::bound_statement(
            self.connection,
            "INSERT INTO members(id,reservation,status,pid,started,socket,launcher) VALUES(?,?,?,?,?,?,?)",
            &parameters,
        )
        .and_then(|mut statement| statement.raw_execute())
        .map_err(failed)?;
        Ok(())
    }

    fn member_row(
        &self,
        id: &Value,
        reservation: Option<&Value>,
    ) -> Result<Option<MemberRow>, BoardError> {
        match reservation {
            None => self.fetch_member("SELECT * FROM members WHERE id=?", &[Bound::Loose(id)]),
            Some(reservation) => self.fetch_member(
                "SELECT * FROM members WHERE id=? AND reservation=?",
                &[Bound::Loose(id), Bound::Loose(reservation)],
            ),
        }
    }

    fn reserve_member(
        &self,
        id: &str,
        reservation: &Value,
        launcher: &str,
    ) -> Result<(), BoardError> {
        self.write(
            "INSERT INTO members(id,reservation,status,pid,started,socket,launcher) VALUES(?,?,'reserved',NULL,NULL,NULL,?)",
            &[Bound::Text(id), Bound::Loose(reservation), Bound::Text(launcher)],
        )
    }

    fn activate_member(
        &self,
        id: &Value,
        launch: &LaunchIdentity,
        socket: &Value,
    ) -> Result<(), BoardError> {
        self.write(
            "UPDATE members SET status='live',pid=?,started=?,socket=? WHERE id=?",
            &[
                Bound::Loose(&launch.pid),
                Bound::Loose(&launch.started),
                Bound::Loose(socket),
                Bound::Loose(id),
            ],
        )
    }

    fn record_launch(&self, id: &Value, launch: &LaunchIdentity) -> Result<(), BoardError> {
        self.write(
            "UPDATE members SET pid=?,started=? WHERE id=?",
            &[
                Bound::Loose(&launch.pid),
                Bound::Loose(&launch.started),
                Bound::Loose(id),
            ],
        )
    }

    fn mark_member_dead_unlaunched(&self, id: &Value) -> Result<(), BoardError> {
        self.write(
            "UPDATE members SET status='dead' WHERE id=?",
            &[Bound::Loose(id)],
        )
    }

    fn set_socket(&self, id: &str, socket: &Value) -> Result<(), BoardError> {
        self.write(
            "UPDATE members SET socket=? WHERE id=?",
            &[Bound::Loose(socket), Bound::Text(id)],
        )
    }
}

/// One parameter of a membership statement: text the board itself
/// supplies (an actor, a bounded member id), or a caller's value bound as
/// Python's `sqlite3` binds it (#2271 round-1 review M1).
enum Bound<'v> {
    Text(&'v str),
    Loose(&'v Value),
}

/// The parameters in order, as Python binds them: the first a caller's
/// value `sqlite3` cannot bind is refused, naming its position in the
/// statement (Python's and the Rust board's statements are the same).
fn bound(parameters: &[Bound<'_>]) -> Result<Vec<SqlValue>, BoardError> {
    parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| match parameter {
            Bound::Text(text) => Ok(SqlValue::Text((*text).to_owned())),
            Bound::Loose(value) => loose(index + 1, value),
        })
        .collect()
}

impl SqliteBoard<'_> {
    /// One statement of Python's SQL with its parameters bound as Python
    /// binds them.
    fn write(&self, sql: &str, parameters: &[Bound<'_>]) -> Result<(), BoardError> {
        let parameters = bound(parameters)?;
        binding::bound_statement(self.connection, sql, &parameters)
            .and_then(|mut statement| statement.raw_execute())
            .map(|_| ())
            .map_err(failed)
    }

    /// The first member row `sql` selects, as `dict(row)`.
    fn fetch_member(
        &self,
        sql: &str,
        parameters: &[Bound<'_>],
    ) -> Result<Option<MemberRow>, BoardError> {
        let parameters = bound(parameters)?;
        let mut statement =
            binding::bound_statement(self.connection, sql, &parameters).map_err(failed)?;
        let mut rows = statement.raw_query();
        rows.next()
            .and_then(|row| row.map(row_dict).transpose())
            .map_err(failed)
    }
}
