//! `BoardMembers` over the SQLite store (#2270, #2271): the `members` rows
//! by Python's SQL (`swarm.py`, `swarm_repository.py`), split out of
//! `repository.rs` to keep each file within its size budget.
use rusqlite::types::Value as SqlValue;
use rusqlite::{OptionalExtension, params};

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
        id: &str,
        reservation: Option<&str>,
    ) -> Result<Option<MemberRow>, BoardError> {
        let found = match reservation {
            None => self
                .connection
                .query_row("SELECT * FROM members WHERE id=?", [id], row_dict),
            Some(reservation) => self.connection.query_row(
                "SELECT * FROM members WHERE id=? AND reservation=?",
                [id, reservation],
                row_dict,
            ),
        };
        found.optional().map_err(failed)
    }

    fn reserve_member(
        &self,
        id: &str,
        reservation: &str,
        launcher: &str,
    ) -> Result<(), BoardError> {
        self.write(
            "INSERT INTO members(id,reservation,status,pid,started,socket,launcher) VALUES(?,?,'reserved',NULL,NULL,NULL,?)",
            params![id, reservation, launcher],
        )
    }

    fn activate_member(
        &self,
        id: &str,
        launch: &LaunchIdentity,
        socket: Option<&str>,
    ) -> Result<(), BoardError> {
        self.write(
            "UPDATE members SET status='live',pid=?,started=?,socket=? WHERE id=?",
            params![launch.pid, launch.started, socket, id],
        )
    }

    fn record_launch(&self, id: &str, launch: &LaunchIdentity) -> Result<(), BoardError> {
        self.write(
            "UPDATE members SET pid=?,started=? WHERE id=?",
            params![launch.pid, launch.started, id],
        )
    }

    fn mark_member_dead_unlaunched(&self, id: &str) -> Result<(), BoardError> {
        self.write("UPDATE members SET status='dead' WHERE id=?", params![id])
    }

    fn set_socket(&self, id: &str, socket: Option<&str>) -> Result<(), BoardError> {
        self.write(
            "UPDATE members SET socket=? WHERE id=?",
            params![socket, id],
        )
    }
}

impl SqliteBoard<'_> {
    /// One statement of Python's SQL with typed parameters.
    fn write(&self, sql: &str, parameters: &[&dyn rusqlite::ToSql]) -> Result<(), BoardError> {
        self.connection
            .execute(sql, parameters)
            .map(|_| ())
            .map_err(failed)
    }
}
