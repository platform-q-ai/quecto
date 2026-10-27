//! Retention sweep: canonical sidecars and orphaned partials only.
use super::*;
/// Another board's concurrent run can grow the shared destination past the
/// sweep's bound; the sweep then works on what it listed, never on presence
/// inferred from a partial listing.
#[test]
fn backup_sweep_listing_overflow_is_not_fatal_and_keeps_retained_sidecars() {
    let temp = tempfile::tempdir().unwrap();
    let to = temp.path().join("snapshots");
    std::fs::create_dir(&to).unwrap();
    let retained = format!("{BOARD}-20261020T100000Z.sqlite");
    std::fs::write(to.join(&retained), b"snapshot").unwrap();
    for suffix in ["-wal", "-shm", "-journal"] {
        std::fs::write(to.join(format!("{retained}{suffix}")), b"").unwrap();
    }
    for n in 0..8 {
        std::fs::write(to.join(format!("other-{n}")), b"").unwrap();
    }
    let adapter = SqliteBoardBackup::new(temp.path().to_owned());
    let limits = BackupLimits {
        entries: 1,
        ..BackupLimits::default()
    };
    let budget = Budget {
        owner: &adapter,
        invocation: adapter.time.elapsed(),
        board: adapter.time.elapsed(),
        limits: &limits,
    };
    let directory = fs::directory(&to).unwrap();
    let mut swept = 0;
    assert_eq!(
        adapter.sweep_sidecars(
            &SweepScope {
                directory: &directory,
                canonical: &to,
                board: BOARD,
                own_partial: &partial_name(BOARD),
            },
            &budget,
            &mut swept
        ),
        Ok(())
    );
    assert_eq!(swept, 0);
    let names = listing(&to);
    for suffix in ["-wal", "-shm", "-journal"] {
        assert!(names.contains(&format!("{retained}{suffix}")));
    }
}
#[test]
fn backup_sweep_skips_odd_canonical_entries_and_continues() {
    use std::os::unix::{
        ffi::OsStringExt,
        fs::{MetadataExt, symlink},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    std::fs::write(to.join("target.txt"), b"foreign").unwrap();
    let symlinked = format!("{BOARD}-20261001T120000Z.sqlite-wal");
    symlink(to.join("target.txt"), to.join(&symlinked)).unwrap();
    let hardlinked = format!("{BOARD}-20261002T120000Z.sqlite-wal");
    std::fs::write(to.join("peer.txt"), b"peer").unwrap();
    std::fs::hard_link(to.join("peer.txt"), to.join(&hardlinked)).unwrap();
    let directory = format!("{BOARD}-20261003T120000Z.sqlite-wal");
    std::fs::create_dir(to.join(&directory)).unwrap();
    let fifo = format!("{BOARD}-20261004T120000Z.sqlite-wal");
    let fifo_path = std::ffi::CString::new(to.join(&fifo).into_os_string().into_vec()).unwrap();
    // SAFETY: a live NUL-terminated path; mkfifo takes no other pointers.
    assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);
    let orphan = format!("{BOARD}-20261009T120000Z.sqlite-wal");
    std::fs::write(to.join(&orphan), b"").unwrap();
    let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.stage, BackupStage::Complete);
    let names = listing(&to);
    for odd in [&symlinked, &hardlinked, &directory, &fifo] {
        assert!(names.contains(odd), "{odd} is left alone");
    }
    assert!(
        !names.contains(&orphan),
        "the later genuine orphan is swept"
    );
    assert_eq!(std::fs::read(to.join("target.txt")).unwrap(), b"foreign");
    assert_eq!(std::fs::metadata(to.join("peer.txt")).unwrap().nlink(), 2);
}
#[test]
fn backup_sweep_runs_when_preflight_admitted_a_full_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    for suffix in ["-wal", "-shm"] {
        std::fs::write(
            to.join(format!("{BOARD}-20261001T120000Z.sqlite{suffix}")),
            b"",
        )
        .unwrap();
    }
    let source_entries = std::fs::read_dir(&root).unwrap().count();
    let limit = listing(&to).len();
    assert_eq!(
        limit, 4,
        "lock + snapshot + two orphans fill the limit exactly"
    );
    assert!(source_entries <= limit);
    let limits = BackupLimits {
        entries: limit,
        ..BackupLimits::default()
    };
    let outcome = back_up_with(
        &root,
        &to,
        "2026-10-25T12:00:00Z",
        Arc::new(SystemBackupFileOps),
        limits,
    );
    assert_eq!(outcome.error, None);
    assert_eq!(
        listing(&to),
        [
            format!(".{BOARD}.backup.lock"),
            format!("{BOARD}-20261020T100000Z.sqlite"),
            format!("{BOARD}-20261025T120000Z.sqlite"),
        ]
    );
}
fn sweep_fixture(temp: &Path) -> (SqliteBoardBackup, PathBuf, BackupLimits) {
    let to = temp.join("snapshots");
    std::fs::create_dir(&to).unwrap();
    (
        SqliteBoardBackup::new(temp.to_owned()),
        to,
        BackupLimits::default(),
    )
}
/// Presence of a snapshot is asked of the directory: a listing that names a
/// retained snapshot's sidecars but not the snapshot itself never sweeps them.
#[test]
fn backup_sweep_asks_the_directory_whether_a_listed_sidecars_snapshot_exists() {
    let temp = tempfile::tempdir().unwrap();
    let (adapter, to, limits) = sweep_fixture(temp.path());
    let retained = format!("{BOARD}-20261020T100000Z.sqlite");
    std::fs::write(to.join(&retained), b"snapshot").unwrap();
    let orphan = format!("{BOARD}-20261001T120000Z.sqlite-wal");
    let mut names = vec![format!("{retained}-journal"), format!("{retained}-wal")];
    names.push(orphan.clone());
    for name in &names {
        std::fs::write(to.join(name), b"").unwrap();
    }
    names.sort();
    let budget = Budget {
        owner: &adapter,
        invocation: adapter.time.elapsed(),
        board: adapter.time.elapsed(),
        limits: &limits,
    };
    let directory = fs::directory(&to).unwrap();
    let mut swept = 0;
    assert_eq!(
        adapter.sweep_listed(
            &SweepScope {
                directory: &directory,
                canonical: &to,
                board: BOARD,
                own_partial: &partial_name(BOARD),
            },
            &budget,
            &names,
            &mut swept
        ),
        Ok(())
    );
    assert_eq!(
        swept, 1,
        "the listed canonical orphan was reached and swept"
    );
    assert_eq!(
        listing(&to),
        [
            retained.clone(),
            format!("{retained}-journal"),
            format!("{retained}-wal"),
        ]
    );
}
#[test]
fn backup_sweep_listing_failure_is_a_retention_failure() {
    let temp = tempfile::tempdir().unwrap();
    let (adapter, to, limits) = sweep_fixture(temp.path());
    std::fs::write(to.join("not-a-directory"), b"").unwrap();
    let budget = Budget {
        owner: &adapter,
        invocation: adapter.time.elapsed(),
        board: adapter.time.elapsed(),
        limits: &limits,
    };
    let file = File::open(to.join("not-a-directory")).unwrap();
    let mut swept = 0;
    assert_eq!(
        adapter.sweep_sidecars(
            &SweepScope {
                directory: &file,
                canonical: &to,
                board: BOARD,
                own_partial: &partial_name(BOARD),
            },
            &budget,
            &mut swept
        ),
        Err(BackupError::RetentionFailed)
    );
}
/// Only entry-level refusals are skipped; a device-level unlink error fails
/// retention after the earlier unlinks are made durable.
#[test]
fn backup_sweep_io_error_fails_retention_after_syncing() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let pruned = format!("{BOARD}-20261020T100000Z.sqlite");
    std::fs::copy(
        to.join(&pruned),
        to.join(format!("{BOARD}-20261020T110000Z.sqlite")),
    )
    .unwrap();
    std::fs::write(to.join(format!("{pruned}-wal")), b"").unwrap();
    let files = CountingSyncs::new(Refuse::WalSidecarsWithIo);
    let outcome = back_up_with(
        &root,
        &to,
        "2026-10-25T12:00:00Z",
        files.clone(),
        BackupLimits::default(),
    );
    assert_eq!(outcome.error, Some(BackupError::RetentionFailed));
    assert_eq!(outcome.stage, BackupStage::Retention);
    assert_eq!(outcome.deleted, 1);
    assert!(listing(&to).contains(&format!("{pruned}-wal")));
    assert_eq!(files.syncs(), 2);
}
/// A canonical partial name exactly as `board` spells one: `.{board}.{v4}.partial`.
fn partial_name(board: &str) -> String {
    format!(".{board}.{}.partial", uuid::Uuid::new_v4())
}
/// #2769: a SIGKILL or OOM mid-backup leaves this board's partial and its
/// sidecars behind; the next run sweeps them under the board's lock.
#[test]
fn backup_retention_sweeps_this_boards_orphaned_partial_and_its_sidecars() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let orphan = partial_name(BOARD);
    std::fs::write(to.join(&orphan), b"half a copy").unwrap();
    for suffix in ["-wal", "-shm", "-journal"] {
        std::fs::write(to.join(format!("{orphan}{suffix}")), b"").unwrap();
    }
    let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.stage, BackupStage::Complete);
    assert_eq!(
        listing(&to),
        [
            format!(".{BOARD}.backup.lock"),
            format!("{BOARD}-20261020T100000Z.sqlite"),
            format!("{BOARD}-20261025T120000Z.sqlite"),
        ]
    );
}
/// Another board's partial may be in flight under that board's lock.
#[test]
fn backup_retention_leaves_another_boards_partial_alone() {
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let foreign = partial_name("fedcba9876543210");
    let mut kept = vec![foreign.clone()];
    for suffix in ["-wal", "-shm", "-journal"] {
        kept.push(format!("{foreign}{suffix}"));
    }
    for name in &kept {
        std::fs::write(to.join(name), b"").unwrap();
    }
    let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(outcome.error, None);
    let names = listing(&to);
    for name in &kept {
        assert!(names.contains(name), "{name} is another board's");
    }
}
/// Only the exact spelling the adapter generates is swept: a lowercase,
/// hyphenated version-4 UUID between `.{board}.` and `.partial`.
#[test]
fn backup_retention_leaves_non_canonical_partial_look_alikes_alone() {
    use std::os::unix::fs::{MetadataExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let root = bootstrapped_board(temp.path());
    let to = temp.path().join("snapshots");
    assert_eq!(back_up(&root, &to, "2026-10-20T10:00:00Z").error, None);
    let id = uuid::Uuid::new_v4();
    let look_alikes = [
        // Not lowercase-hyphenated.
        format!(
            ".{BOARD}.{}.partial",
            id.hyphenated().to_string().to_uppercase()
        ),
        format!(".{BOARD}.{}.partial", id.simple()),
        format!(".{BOARD}.{}.partial", id.braced()),
        format!(".{BOARD}.{}.partial", id.urn()),
        // Well formed, but not a version-4 UUID.
        format!(".{BOARD}.6ba7b810-9dad-11d1-80b4-00c04fd430c8.partial"),
        format!(".{BOARD}.00000000-0000-0000-0000-000000000000.partial"),
        // Wrong frame around a canonical UUID.
        format!("{BOARD}.{id}.partial"),
        format!(".{BOARD}.{id}.partial.tmp"),
        format!(".{BOARD}.{id}.partial-walx"),
        format!(".{BOARD}.{id}.partial-wal-shm"),
        format!(".{BOARD}.{id}.partial.wal"),
        format!(".{BOARD}.{id}x.partial"),
        format!(".{BOARD}..{id}.partial"),
        format!("..{BOARD}.{id}.partial"),
        format!(".{}.{id}.partial", BOARD.to_uppercase()),
        format!(".{BOARD}.backup.partial"),
    ];
    for name in &look_alikes {
        std::fs::write(to.join(name), b"").unwrap();
    }
    // Canonical names that are not single-link regular files.
    std::fs::write(to.join("target.txt"), b"foreign").unwrap();
    let symlinked = partial_name(BOARD);
    symlink(to.join("target.txt"), to.join(&symlinked)).unwrap();
    std::fs::write(to.join("peer.txt"), b"peer").unwrap();
    let hardlinked = partial_name(BOARD);
    std::fs::hard_link(to.join("peer.txt"), to.join(&hardlinked)).unwrap();
    let directory = partial_name(BOARD);
    std::fs::create_dir(to.join(&directory)).unwrap();
    let outcome = back_up(&root, &to, "2026-10-25T12:00:00Z");
    assert_eq!(outcome.error, None);
    assert_eq!(outcome.stage, BackupStage::Complete);
    let names = listing(&to);
    for name in look_alikes
        .iter()
        .chain([&symlinked, &hardlinked, &directory])
    {
        assert!(names.contains(name), "{name} is left alone");
    }
    assert_eq!(std::fs::read(to.join("target.txt")).unwrap(), b"foreign");
    assert_eq!(std::fs::metadata(to.join("peer.txt")).unwrap().nlink(), 2);
}
/// The current run's own partial and its sidecars are never swept, even if
/// a listing still names them.
#[test]
fn backup_sweep_never_touches_the_current_runs_own_partial() {
    let temp = tempfile::tempdir().unwrap();
    let (adapter, to, limits) = sweep_fixture(temp.path());
    let own = partial_name(BOARD);
    let orphan = partial_name(BOARD);
    let mut names = vec![own.clone(), format!("{own}-journal"), orphan.clone()];
    for name in &names {
        std::fs::write(to.join(name), b"").unwrap();
    }
    names.sort();
    let budget = Budget {
        owner: &adapter,
        invocation: adapter.time.elapsed(),
        board: adapter.time.elapsed(),
        limits: &limits,
    };
    let directory = fs::directory(&to).unwrap();
    let mut swept = 0;
    assert_eq!(
        adapter.sweep_listed(
            &SweepScope {
                directory: &directory,
                canonical: &to,
                board: BOARD,
                own_partial: &own,
            },
            &budget,
            &names,
            &mut swept
        ),
        Ok(())
    );
    assert_eq!(swept, 1, "only the orphan is swept");
    let mut expected = vec![own.clone(), format!("{own}-journal")];
    expected.sort();
    assert_eq!(listing(&to), expected);
}
