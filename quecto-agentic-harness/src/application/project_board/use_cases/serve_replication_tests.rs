//! A primary's batches (epic #2705 B11) carry its log's rows and every
//! admin row; a store or admin rows that cannot be read fail the batch.
use std::sync::Mutex;

use super::*;
use crate::application::project_board::ports::TokenError;
use crate::application::project_board::ports::replication::AdminToken;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

fn token(name: &str) -> AdminToken {
    AdminToken {
        name: Slug::parse(name).unwrap(),
        created: Timestamp::parse("2026-10-26T00:00:00Z").unwrap(),
        digest: "0".repeat(64),
    }
}

struct Log(Result<(), BoardStoreError>);

impl ReplicationLog for Log {
    fn replica_batch(
        &self,
        after: u64,
        _: u64,
        _: BatchLimit,
    ) -> Result<ReplicaBatch, BoardStoreError> {
        self.0.clone()?;
        Ok(ReplicaBatch {
            after,
            chain_after: None,
            objects: vec![],
            revisions: vec![],
            epochs: vec![],
            meta: None,
            tokens: vec![],
            tip: 0,
            more: false,
        })
    }
}

struct Admin(Mutex<Result<Vec<AdminToken>, TokenError>>);

impl AdminRows for Admin {
    fn export(&self) -> Result<Vec<AdminToken>, TokenError> {
        self.0.lock().unwrap().clone()
    }

    fn replace(&self, rows: &[AdminToken]) -> Result<(), TokenError> {
        *self.0.lock().unwrap() = Ok(rows.to_vec());
        Ok(())
    }
}

#[test]
fn a_primary_serves_its_log_with_its_admin_rows() {
    let admin = Arc::new(Admin(Mutex::new(Ok(vec![token("laptop")]))));
    let served = ServeReplication::new(Arc::new(Log(Ok(()))), admin);
    let batch = served.batch(4, 0, BatchLimit::MOST).unwrap();
    assert_eq!((batch.after, batch.tokens), (4, vec![token("laptop")]));
}

#[test]
fn a_batch_fails_when_its_log_or_admin_rows_cannot_be_read() {
    let down = BoardStoreError::Unavailable("the disk is gone".into());
    let admin = Arc::new(Admin(Mutex::new(Ok(vec![]))));
    let served = ServeReplication::new(Arc::new(Log(Err(down.clone()))), admin);
    let failed = served.batch(0, 0, BatchLimit::MOST);
    assert_eq!(
        failed,
        Err(ServeError::Board(ProjectBoardError::Store(down)))
    );
    let unread = TokenError::Unavailable("the tokens file is gone".into());
    let admin = Arc::new(Admin(Mutex::new(Err(unread))));
    let served = ServeReplication::new(Arc::new(Log(Ok(()))), admin);
    assert!(matches!(
        served.batch(0, 0, BatchLimit::MOST),
        Err(ServeError::Board(ProjectBoardError::Store(
            BoardStoreError::Unavailable(why)
        ))) if why.contains("tokens file")
    ));
}
