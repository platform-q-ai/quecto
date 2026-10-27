//! A crash between publication's link and unlink leaves the snapshot and the
//! partial on one inode; the next run repairs exactly that pair.
use super::*;
use std::os::unix::fs::MetadataExt;

/// A SIGKILL after `linkat(partial, snapshot)` and before the partial's unlink.
fn interrupted_publication(to: &Path, snapshot: &str, peer: &str) {
    std::fs::hard_link(to.join(snapshot), to.join(peer)).unwrap();
    assert_eq!(std::fs::metadata(to.join(snapshot)).unwrap().nlink(), 2);
}
#[test]
fn backup_repairs_a_publication_interrupted_between_link_and_unlink() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let snapshot = format!("{BOARD}-20261020T100000Z.sqlite");
    let before = std::fs::read(to.join(&snapshot)).unwrap();
    let partial = format!(".{BOARD}.{}.partial", uuid::Uuid::new_v4());
    interrupted_publication(&to, &snapshot, &partial);
    let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.stage, BackupStage::Complete);
    assert_eq!((outcome.retained, outcome.deleted), (2, 0));
    assert_eq!(
        listing(&to),
        [
            format!(".{BOARD}.backup.lock"),
            snapshot.clone(),
            format!("{BOARD}-20261025T120000Z.sqlite"),
        ]
    );
    assert_eq!(std::fs::metadata(to.join(&snapshot)).unwrap().nlink(), 1);
    assert_eq!(std::fs::read(to.join(&snapshot)).unwrap(), before);
}
/// Only the exact pair is repaired: any other peer of a 2-link snapshot
/// stays refused, and both names are left as they were.
#[test]
fn backup_still_refuses_a_two_link_snapshot_with_any_other_peer() {
    let id = uuid::Uuid::new_v4();
    let peers = [
        "notes.sqlite".to_owned(),
        format!(".{BOARD}.{id}.partial.tmp"),
        format!(".{BOARD}.{}.partial", id.simple()),
        format!(".{BOARD}.{id}.partial-wal"),
        format!(".fedcba9876543210.{id}.partial"),
        format!("{BOARD}-20261019T100000Z.sqlite"),
    ];
    for peer in peers {
        let temp = tempfile::tempdir().unwrap();
        let root = bootstrapped_board(temp.path());
        let to = temp.path().join("snapshots");
        assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
        let snapshot = format!("{BOARD}-20261020T100000Z.sqlite");
        interrupted_publication(&to, &snapshot, &peer);
        let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
        assert_eq!(outcome.error, Some(BackupError::UnsafePath), "{peer}");
        assert!(listing(&to).contains(&peer), "{peer} is left alone");
        assert_eq!(std::fs::metadata(to.join(&snapshot)).unwrap().nlink(), 2);
    }
}
/// Three names on one inode is not an interrupted publication.
#[test]
fn backup_still_refuses_a_canonical_pair_with_a_third_link() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let snapshot = format!("{BOARD}-20261020T100000Z.sqlite");
    let partial = format!(".{BOARD}.{}.partial", uuid::Uuid::new_v4());
    interrupted_publication(&to, &snapshot, &partial);
    std::fs::hard_link(to.join(&snapshot), to.join("third")).unwrap();
    let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(outcome.error, Some(BackupError::UnsafePath));
    assert!(listing(&to).contains(&partial));
    assert_eq!(std::fs::metadata(to.join(&snapshot)).unwrap().nlink(), 3);
}
/// A canonical partial that is a symlink to the snapshot is not a peer.
#[test]
fn backup_still_refuses_when_the_canonical_partial_is_a_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let snapshot = format!("{BOARD}-20261020T100000Z.sqlite");
    std::fs::hard_link(to.join(&snapshot), temp.path().join("outside")).unwrap();
    let partial = format!(".{BOARD}.{}.partial", uuid::Uuid::new_v4());
    std::os::unix::fs::symlink(to.join(&snapshot), to.join(&partial)).unwrap();
    let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(outcome.error, Some(BackupError::UnsafePath));
    assert!(listing(&to).contains(&partial));
    assert_eq!(std::fs::metadata(to.join(&snapshot)).unwrap().nlink(), 2);
}
/// The repair runs only under the board lock: while another holder has it,
/// the run refuses busy and the pair is left exactly as it was.
#[test]
fn backup_leaves_an_interrupted_pair_alone_while_the_board_lock_is_held() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let snapshot = format!("{BOARD}-20261020T100000Z.sqlite");
    let partial = format!(".{BOARD}.{}.partial", uuid::Uuid::new_v4());
    interrupted_publication(&to, &snapshot, &partial);
    let directory = fs::directory(&to).unwrap();
    // Generous: a forked test child may briefly hold a copy of a lock fd.
    let _held = fs::lock(&directory, BOARD, Duration::from_secs(5)).unwrap();
    let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(outcome.error, Some(BackupError::BusyDeadline));
    let names = listing(&to);
    assert!(names.contains(&snapshot) && names.contains(&partial));
    assert_eq!(std::fs::metadata(to.join(&snapshot)).unwrap().nlink(), 2);
}
/// #2773: a lock held briefly by someone else is waited out within
/// `limits.busy`; the run completes instead of reporting busy. The holder lets
/// go only 100 ms after the run's own `lock()` has opened the lock file (the
/// `between_open_and_flock` seam), so the run's flock meets a held lock.
#[test]
fn backup_waits_out_a_brief_hold_of_the_board_lock() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let directory = fs::directory(&to).unwrap();
    let held = fs::lock(&directory, BOARD, Duration::from_secs(5)).unwrap();
    let (release, signal) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        if signal.recv().is_ok() {
            std::thread::sleep(Duration::from_millis(100));
        }
        drop(held);
    });
    fs::between_open_and_flock::set(move || release.send(()).unwrap());
    let outcome = back_up_with(
        &root,
        &to,
        "2026-10-25T12:00:00Z",
        Arc::new(SystemBackupFileOps),
        BackupLimits {
            busy: Duration::from_secs(5),
            ..BackupLimits::default()
        },
    );
    // Drops an unrun hook's sender, so the holder never waits for ever.
    fs::between_open_and_flock::set(|| {});
    holder.join().unwrap();
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.stage, BackupStage::Complete);
}
