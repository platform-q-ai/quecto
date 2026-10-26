//! Contract for [`ReplicaStore`] (epic #2705 B11) on a follower's SQLite
//! board: a batch is applied whole or not at all; a revision whose rows do
//! not make its chain, an object out of order, or a batch after a place
//! the store is not at is refused, nothing applied; the batch that is the
//! next one leaves a copy of the primary's rows. The provenance row and
//! the epochs are allowlists: a binding is never overwritten (nor with
//! none), an epoch held is never changed, and none is taken past a gap.
use quecto::application::project_board::ports::feed::BatchLimit;
use quecto::application::project_board::ports::replication::{
    ReplicaEpoch, ReplicaError, ReplicaStore, ReplicationLog,
};

use super::board_feed::wrote;
use super::replication_log::{BOARD, dump, server, server_of};

fn board_id(file: &std::path::Path) -> Option<String> {
    let c = rusqlite::Connection::open(file).unwrap();
    c.query_row("SELECT board_id FROM history_meta WHERE id=1", [], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn a_binding_is_never_overwritten_nor_with_none() {
    let primary = server();
    wrote(&primary.ops, 1, true, 1);
    let batch = primary.ops.replica_batch(0, 0, BatchLimit::MOST).unwrap();
    // A follower bound to another board refuses it, its binding kept.
    let follower = server_of(Some("fedcba9876543210"));
    assert!(matches!(
        follower.ops.apply(&batch),
        Err(ReplicaError::Invalid(_))
    ));
    // A primary row naming no board is refused by a bound follower.
    let follower = server();
    let mut unbound = batch.clone();
    unbound.meta.as_mut().unwrap().board_id = None;
    assert!(matches!(
        follower.ops.apply(&unbound),
        Err(ReplicaError::Invalid(_))
    ));
    assert_eq!(
        follower.ops.position().unwrap().revision,
        0,
        "nothing applied"
    );
    // An unbound follower takes the primary's binding; a bound one keeps its own.
    let fresh = server_of(None);
    fresh.ops.apply(&batch).unwrap();
    assert_eq!(board_id(&fresh.file).as_deref(), Some(BOARD));
    follower.ops.apply(&batch).unwrap();
    assert_eq!(board_id(&follower.file).as_deref(), Some(BOARD));
}

/// B11 review 2: the provenance held is the primary's, field for field: a
/// board of another algorithm or import source is another history.
#[test]
fn a_provenance_held_otherwise_is_a_fork() {
    let primary = server();
    wrote(&primary.ops, 1, true, 1);
    let batch = primary.ops.replica_batch(0, 0, BatchLimit::MOST).unwrap();
    let follower = server();
    follower.ops.apply(&batch).unwrap();
    let mut next = primary
        .ops
        .replica_batch(2, u64::MAX, BatchLimit::MOST)
        .unwrap();
    next.meta.as_mut().unwrap().algorithm = "sha1".into();
    assert!(matches!(
        follower.ops.apply(&next),
        Err(ReplicaError::Diverged(_))
    ));
    let mut next = primary
        .ops
        .replica_batch(2, u64::MAX, BatchLimit::MOST)
        .unwrap();
    next.meta.as_mut().unwrap().source_remote = Some("elsewhere".into());
    assert!(matches!(
        follower.ops.apply(&next),
        Err(ReplicaError::Diverged(_))
    ));
    let same = primary
        .ops
        .replica_batch(2, u64::MAX, BatchLimit::MOST)
        .unwrap();
    follower.ops.apply(&same).unwrap();
}

/// B11 review 2: a follower's operations store refuses every write.
#[test]
fn a_standby_store_refuses_every_write() {
    use quecto::application::project_board::ports::{BoardOpStore, OpWrite, WriteId};
    let follower = std::sync::Arc::new(server_of(None).ops);
    let standby = follower.standby();
    let write = OpWrite {
        write_id: WriteId::parse("00000000-0000-4000-8000-000000000001").unwrap(),
        request: "cli\nbootstrap".into(),
        committer: super::board_feed::identity(),
        principal: "local".into(),
    };
    let mut ran = false;
    let refused = standby.transact(&write, &mut |_| {
        ran = true;
        Ok(String::new())
    });
    assert!(refused.is_err());
    assert!(!ran, "the op never runs");
}

#[test]
fn an_epoch_held_is_never_changed_and_none_is_taken_past_a_gap() {
    let primary = server();
    wrote(&primary.ops, 1, true, 1);
    let batch = primary.ops.replica_batch(0, 0, BatchLimit::MOST).unwrap();
    let follower = server();
    follower.ops.apply(&batch).unwrap();
    let mut next = primary
        .ops
        .replica_batch(2, u64::MAX, BatchLimit::MOST)
        .unwrap();
    let before = dump(&follower.file);
    next.epochs[0].reason = "rewritten".into();
    assert!(matches!(
        follower.ops.apply(&next),
        Err(ReplicaError::Diverged(_))
    ));
    let mut gap = primary
        .ops
        .replica_batch(2, u64::MAX, BatchLimit::MOST)
        .unwrap();
    gap.epochs.push(ReplicaEpoch {
        epoch: 3,
        from_revision: 2,
        reason: "past a gap".into(),
        at: 0,
    });
    assert!(matches!(
        follower.ops.apply(&gap),
        Err(ReplicaError::Invalid(_))
    ));
    assert!(dump(&follower.file) == before, "nothing applied");
    let mut opened = primary
        .ops
        .replica_batch(2, u64::MAX, BatchLimit::MOST)
        .unwrap();
    opened.epochs.push(ReplicaEpoch {
        epoch: 2,
        from_revision: 2,
        reason: "promoted".into(),
        at: 0,
    });
    follower.ops.apply(&opened).unwrap();
}

#[test]
fn a_store_applies_a_batch_whole_or_not_at_all() {
    let (primary, follower) = (server(), server());
    wrote(&primary.ops, 1, true, 3);
    let batch = primary.ops.replica_batch(0, 0, BatchLimit::MOST).unwrap();
    // A revision whose rows do not hash to its chain.
    let mut tampered = batch.clone();
    tampered.revisions[2].message = "board: not what was written".into();
    let before = dump(&follower.file);
    assert!(matches!(
        follower.ops.apply(&tampered),
        Err(ReplicaError::Invalid(_))
    ));
    // An object out of the primary's order.
    let mut gapped = batch.clone();
    gapped.objects.remove(0);
    assert!(matches!(
        follower.ops.apply(&gapped),
        Err(ReplicaError::Invalid(_))
    ));
    // A batch after a place the store is not at.
    let mut elsewhere = batch.clone();
    elsewhere.after = 1;
    assert!(matches!(
        follower.ops.apply(&elsewhere),
        Err(ReplicaError::Diverged(_))
    ));
    assert!(dump(&follower.file) == before, "nothing applied");
    let position = follower.ops.apply(&batch).unwrap();
    assert_eq!(position.revision, 4);
    assert_eq!(position, follower.ops.position().unwrap());
    assert!(dump(&follower.file) == dump(&primary.file));
}
