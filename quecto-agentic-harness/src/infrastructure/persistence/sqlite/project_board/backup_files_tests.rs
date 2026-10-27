use super::*;
use std::{
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
};
#[test]
fn backup_syscalls_require_owned_identity_and_never_replace_completed_name() {
    let temp = tempfile::tempdir().unwrap();
    let directory = anchored::directory(temp.path()).unwrap();
    let operations = SystemBackupFileOps;
    let mut partial = operations
        .create_partial(&directory, ".owned.partial")
        .unwrap();
    assert_eq!(partial.metadata().unwrap().mode() & 0o777, 0o600);
    partial.write_all(b"new verified data").unwrap();
    let record = AdmittedBackupFile {
        name: ".owned.partial".into(),
        identity: anchored::identity(&partial.metadata().unwrap()).unwrap(),
    };
    std::fs::write(temp.path().join("completed.sqlite"), b"previous good").unwrap();
    assert_eq!(
        operations
            .publish_noreplace(&directory, &record, "completed.sqlite")
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        std::fs::read(temp.path().join("completed.sqlite")).unwrap(),
        b"previous good"
    );
    operations.sync_file(&partial).unwrap();
    operations
        .publish_noreplace(&directory, &record, "new.sqlite")
        .unwrap();
    operations.sync_directory(&directory).unwrap();
    assert_eq!(
        std::fs::read(temp.path().join("new.sqlite")).unwrap(),
        b"new verified data"
    );
    assert_eq!(
        std::fs::metadata(temp.path().join("new.sqlite"))
            .unwrap()
            .nlink(),
        1
    );
    let mut completed: Vec<_> = std::fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    completed.sort();
    assert_eq!(completed, ["completed.sqlite", "new.sqlite"]);
}
#[test]
fn backup_syscalls_refuse_hardlink_alias_and_detect_directory_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let to = temp.path().join("private");
    std::fs::create_dir(&to).unwrap();
    std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o700)).unwrap();
    let directory = anchored::directory(&to).unwrap();
    let file = SystemBackupFileOps
        .create_partial(&directory, "owned.partial")
        .unwrap();
    let record = AdmittedBackupFile {
        name: "owned.partial".into(),
        identity: anchored::identity(&file.metadata().unwrap()).unwrap(),
    };
    std::fs::hard_link(to.join("owned.partial"), to.join("foreign-alias")).unwrap();
    assert!(
        SystemBackupFileOps
            .prune_admitted(&directory, &record)
            .is_err()
    );
    std::fs::rename(&to, temp.path().join("original")).unwrap();
    std::fs::create_dir(&to).unwrap();
    assert!(anchored::stable_directory(&directory, &to).is_err());
    assert_eq!(std::fs::read_dir(&to).unwrap().count(), 0);
}

#[test]
fn backup_destination_syncs_held_parent_for_new_leaf_and_existing_retry() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("boards");
    std::fs::create_dir(&source).unwrap();
    let to = temp.path().join("private");
    let (new, canonical) = anchored::private_destination(&to, &source).unwrap();
    assert_eq!(canonical, to);
    assert_eq!(new.metadata().unwrap().mode() & 0o777, 0o700);
    drop(new);
    let (retry, same) = anchored::private_destination(&to, &source).unwrap();
    assert_eq!(same, to);
    anchored::stable_directory(&retry, &to).unwrap();
    assert_eq!(std::fs::read_dir(&to).unwrap().count(), 0);
}
#[test]
fn backup_destination_parent_sync_error_refuses_before_any_partial() {
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    let temp = tempfile::tempdir().unwrap();
    let directory = anchored::directory(temp.path()).unwrap();
    let (socket, _peer) = UnixStream::pair().unwrap();
    let owned: OwnedFd = socket.into();
    let parent = File::from(owned);
    // An owned socket produces a real fsync EINVAL. This probes the exact
    // helper's syscall-error propagation, not a fake directory-I/O failure.
    let error =
        anchored::durable_destination(&parent, directory, temp.path().to_owned()).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::EINVAL));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
/// A lock file swapped under its name between open and flock is not the
/// board's lock: two runs could otherwise each hold a different inode.
#[test]
fn backup_lock_refuses_an_inode_its_name_no_longer_names() {
    let temp = tempfile::tempdir().unwrap();
    let directory = anchored::directory(temp.path()).unwrap();
    let board = "0123456789abcdef";
    let lock = temp.path().join(anchored::lock_name(board));
    let stale = anchored::open_lock(&directory, board).unwrap();
    std::fs::rename(&lock, temp.path().join("aside")).unwrap();
    let current = anchored::open_lock(&directory, board).unwrap();
    assert_eq!(
        anchored::hold(&directory, board, stale).unwrap_err().kind(),
        io::ErrorKind::InvalidInput,
        "the renamed-aside inode is not the board lock"
    );
    let held = anchored::hold(&directory, board, current).unwrap();
    let named = std::fs::symlink_metadata(&lock).unwrap();
    assert_eq!(
        (
            held.metadata().unwrap().ino(),
            held.metadata().unwrap().dev()
        ),
        (named.ino(), named.dev())
    );
}
/// The same holds for a lock name unlinked between open and flock.
#[test]
fn backup_lock_refuses_an_unlinked_inode() {
    let temp = tempfile::tempdir().unwrap();
    let directory = anchored::directory(temp.path()).unwrap();
    let board = "0123456789abcdef";
    let stale = anchored::open_lock(&directory, board).unwrap();
    std::fs::remove_file(temp.path().join(anchored::lock_name(board))).unwrap();
    assert!(anchored::hold(&directory, board, stale).is_err());
}
/// Once the old holder is gone, the next run locks the file the name now names.
#[test]
fn backup_lock_after_a_rename_aside_takes_the_newly_named_file() {
    let temp = tempfile::tempdir().unwrap();
    let directory = anchored::directory(temp.path()).unwrap();
    let board = "0123456789abcdef";
    let lock = temp.path().join(anchored::lock_name(board));
    let first = anchored::lock(&directory, board).unwrap();
    std::fs::rename(&lock, temp.path().join("aside")).unwrap();
    drop(first);
    let next = anchored::lock(&directory, board).unwrap();
    let named = std::fs::symlink_metadata(&lock).unwrap();
    assert_eq!(next.metadata().unwrap().ino(), named.ino());
    assert_ne!(
        std::fs::metadata(temp.path().join("aside")).unwrap().ino(),
        named.ino()
    );
}
/// `lock()` itself re-checks the name after its flock: a swap inside its own
/// open-to-flock window is refused, not held.
#[test]
fn backup_lock_refuses_a_name_swapped_inside_its_own_window() {
    let temp = tempfile::tempdir().unwrap();
    let directory = anchored::directory(temp.path()).unwrap();
    let board = "0123456789abcdef";
    let lock = temp.path().join(anchored::lock_name(board));
    let aside = temp.path().join("aside");
    let swap_lock = lock.clone();
    anchored::between_open_and_flock::set(move || {
        std::fs::rename(&swap_lock, &aside).unwrap();
        std::fs::write(&swap_lock, b"").unwrap();
    });
    assert_eq!(
        anchored::lock(&directory, board).unwrap_err().kind(),
        io::ErrorKind::InvalidInput,
        "the flocked inode is no longer the board lock"
    );
    assert!(lock.exists(), "the swapped-in name is left alone");
}
