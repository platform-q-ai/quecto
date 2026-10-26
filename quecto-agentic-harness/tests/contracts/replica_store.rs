//! Contract for [`ReplicaStore`] (epic #2705 B11) on a follower's SQLite
//! board: a batch is applied whole or not at all; a revision whose rows do
//! not make its chain, an object out of order, or a batch after a place
//! the store is not at is refused, nothing applied; the batch that is the
//! next one leaves a copy of the primary's rows.
use quecto::application::project_board::ports::feed::BatchLimit;
use quecto::application::project_board::ports::replication::{
    ReplicaError, ReplicaStore, ReplicationLog,
};

use super::board_feed::wrote;
use super::replication_log::{dump, server};

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
