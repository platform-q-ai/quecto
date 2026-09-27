// #2243 review: where a rename cannot replace the file (a mount point, a
// sticky directory) or cannot keep its owner, the file is written in place,
// as a plain write would, and the caller says so; long names still fit.

use super::*;
use std::os::unix::fs::{MetadataExt, symlink};
use tempfile::TempDir;

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn refusing_rename(kind: io::ErrorKind) -> Steps<'static> {
    let rename: fn(&Path, &Path) -> io::Result<()> = match kind {
        io::ErrorKind::ResourceBusy => |_, _| Err(io::Error::from(io::ErrorKind::ResourceBusy)),
        io::ErrorKind::CrossesDevices => |_, _| Err(io::Error::from(io::ErrorKind::CrossesDevices)),
        io::ErrorKind::PermissionDenied => {
            |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
        _ => |_, _| Err(io::Error::other("rename failed (injected)")),
    };
    Steps {
        rename,
        ..Steps::real()
    }
}

/// M1, M2: a rename the system refuses (EBUSY on a bind-mounted file,
/// EXDEV, EPERM in a sticky directory) falls back to writing in place.
#[test]
fn a_refused_rename_writes_in_place_and_leaves_no_temp_file() {
    for kind in [
        io::ErrorKind::ResourceBusy,
        io::ErrorKind::CrossesDevices,
        io::ErrorKind::PermissionDenied,
    ] {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("f.txt");
        std::fs::write(&target, "a longer old text").unwrap();
        let inode = std::fs::metadata(&target).unwrap().ino();
        let written = replace_contents_with(&target, b"new", refusing_rename(kind)).unwrap();
        assert_eq!(
            written,
            Written::InPlace(InPlace::RenameRefused { leftover: None }),
            "{kind:?}"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"new", "{kind:?}");
        assert_eq!(std::fs::metadata(&target).unwrap().ino(), inode);
        assert_eq!(names(tmp.path()), ["f.txt"], "{kind:?}");
    }
}

#[test]
fn any_other_rename_failure_leaves_the_file_as_it_was() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "old").unwrap();
    let failure =
        replace_contents_with(&target, b"new", refusing_rename(io::ErrorKind::Other)).unwrap_err();
    assert_eq!(failure.file, FileState::Unchanged);
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    assert_eq!(names(tmp.path()), ["f.txt"]);
}

/// A new file has nothing to write in place: a refused rename is an error.
#[test]
fn a_refused_rename_of_a_new_file_is_an_error_and_leaves_nothing() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("new.txt");
    let failure =
        replace_contents_with(&target, b"x", refusing_rename(io::ErrorKind::ResourceBusy))
            .unwrap_err();
    assert_eq!(failure.error.kind(), io::ErrorKind::ResourceBusy);
    assert!(names(tmp.path()).is_empty());
}

fn own(file: &File) -> Owner {
    Owner::of(&file.metadata().unwrap())
}

/// M3: the owner is kept when it already matches, with no chown at all.
#[test]
fn a_matching_owner_needs_no_chown() {
    let tmp = TempDir::new().unwrap();
    let file = File::create(tmp.path().join("t")).unwrap();
    let want = own(&file);
    let chown: Chown = |_, _, _| panic!("no chown for a matching owner");
    assert!(keep_owner(&file, want, chown).unwrap());
}

/// M3: another user's file cannot be given back by an unprivileged writer:
/// not kept, so the caller writes in place. The whole chown is tried first
/// (it works for root), then the group alone.
#[test]
fn another_users_owner_that_cannot_be_given_back_is_not_kept() {
    let tmp = TempDir::new().unwrap();
    let file = File::create(tmp.path().join("t")).unwrap();
    let ours = own(&file);
    let want = Owner {
        uid: ours.uid.wrapping_add(4242),
        gid: ours.gid.wrapping_add(4242),
    };
    thread_local! {
        static CALLS: std::cell::RefCell<Vec<(Option<u32>, Option<u32>)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
    let chown: Chown = |_, uid, gid| {
        CALLS.with(|calls| calls.borrow_mut().push((uid, gid)));
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    };
    assert!(!keep_owner(&file, want, chown).unwrap());
    let calls = CALLS.with(|calls| calls.borrow().clone());
    assert_eq!(
        calls,
        [(Some(want.uid), Some(want.gid)), (None, Some(want.gid))]
    );
    assert_eq!(own(&file), ours, "nothing changed");
}

/// M3: our own file in a group we may not give: not kept either.
#[test]
fn a_group_that_cannot_be_given_is_not_kept() {
    let tmp = TempDir::new().unwrap();
    let file = File::create(tmp.path().join("t")).unwrap();
    let ours = own(&file);
    let want = Owner {
        uid: ours.uid,
        gid: ours.gid.wrapping_add(4242),
    };
    let chown: Chown = |_, uid, _| {
        assert_eq!(uid, None, "our own uid is not re-set");
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    };
    assert!(!keep_owner(&file, want, chown).unwrap());
}

/// M3: when the owner cannot be kept, the file is written in place (its
/// owner, group and mode stay), and the caller says so.
#[test]
fn an_owner_that_cannot_be_kept_writes_in_place() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "old").unwrap();
    let inode = std::fs::metadata(&target).unwrap().ino();
    let steps = Steps {
        owner_of: |meta| {
            let owner = Owner::of(meta);
            Owner {
                gid: owner.gid.wrapping_add(4242),
                ..owner
            }
        },
        chown: |_, _, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        ..Steps::real()
    };
    let written = replace_contents_with(&target, b"new", steps).unwrap();
    assert_eq!(
        written,
        Written::InPlace(InPlace::OwnerNotKept { leftover: None })
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    assert_eq!(std::fs::metadata(&target).unwrap().ino(), inode);
    assert_eq!(names(tmp.path()), ["f.txt"]);
}

/// M4: the temporary name stays within the name limit for a long name.
#[test]
fn a_long_name_is_created_and_replaced() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("é".repeat(120));
    assert_eq!(target.file_name().unwrap().len(), 240);
    assert_eq!(replace_contents(&target, b"one").unwrap(), Written::Created);
    assert_eq!(
        replace_contents(&target, b"two").unwrap(),
        Written::Replaced
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"two");
    assert_eq!(names(tmp.path()).len(), 1);
}

/// L9: a FIFO swapped in for the file is not written (nor waited on).
#[test]
fn a_fifo_is_refused_without_blocking() {
    let tmp = TempDir::new().unwrap();
    let fifo = tmp.path().join("pipe");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: `path` is a valid NUL-terminated C string for the call.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let failure = replace_contents(&fifo, b"x").unwrap_err();
    assert_eq!(failure.file, FileState::Unchanged);
}

/// L9: a file swapped between the look and the open is not written.
#[test]
fn a_file_swapped_before_the_open_is_refused() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "old").unwrap();
    let meta = std::fs::metadata(&target).unwrap();
    // The old file lives on under another name, so the new one is another
    // inode.
    std::fs::rename(&target, tmp.path().join("kept")).unwrap();
    std::fs::write(&target, "swapped").unwrap();
    let error = open_existing(&target, &meta).unwrap_err();
    assert!(error.to_string().contains("changed"), "{error}");
    assert_eq!(std::fs::read(&target).unwrap(), b"swapped");
}

/// Round 2 L1: a temporary file that cannot be removed (an append-only
/// directory refuses the unlink, as it refused the rename) is named.
#[test]
fn a_temp_file_that_cannot_be_removed_is_named() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "old").unwrap();
    let steps = Steps {
        remove: |_| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        ..refusing_rename(io::ErrorKind::PermissionDenied)
    };
    let written = replace_contents_with(&target, b"new", steps).unwrap();
    let Written::InPlace(InPlace::RenameRefused {
        leftover: Some(leftover),
    }) = written
    else {
        panic!("expected a named leftover, got {written:?}");
    };
    assert_eq!(leftover.parent(), Some(tmp.path()));
    assert!(leftover.exists(), "it is really there");
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
}

/// Round 2 L2: a full disk or quota while the temporary file is written
/// leaves the file as it was, and says which.
#[test]
fn a_full_disk_while_staging_leaves_the_file_and_says_so() {
    for kind in [io::ErrorKind::StorageFull, io::ErrorKind::QuotaExceeded] {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("f.txt");
        std::fs::write(&target, "old").unwrap();
        let steps = Steps {
            after_temp: Box::new(move |_| Err(io::Error::from(kind))),
            ..Steps::real()
        };
        let failure = replace_contents_with(&target, b"new", steps).unwrap_err();
        assert_eq!(failure.file, FileState::Unchanged);
        assert_eq!(failure.error.kind(), kind);
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert_eq!(names(tmp.path()), ["f.txt"]);
    }
}

/// Round 2 nit: a new file that cannot be created is said to be absent.
#[test]
fn a_new_file_that_cannot_be_created_is_absent() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("missing-dir/new.txt");
    let failure = replace_contents(&target, b"x").unwrap_err();
    assert_eq!(failure.file, FileState::Absent);
}

/// Swarm review F1: a write through a link to nothing stages its temporary
/// file beside the link's target and renames it in: a failed write leaves
/// no target and no temporary file, and the link stays.
#[test]
fn a_failed_write_through_a_dangling_link_leaves_nothing() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("real")).unwrap();
    std::fs::create_dir(tmp.path().join("links")).unwrap();
    let link = tmp.path().join("links/f.txt");
    symlink("../real/f.txt", &link).unwrap();
    let mut staged_in = None;
    let steps = Steps {
        after_temp: Box::new(|temp| {
            staged_in = temp.parent().map(Path::to_path_buf);
            Err(io::Error::from(io::ErrorKind::StorageFull))
        }),
        ..Steps::real()
    };
    let failure = replace_contents_with(&link, b"new text", steps).unwrap_err();
    assert_eq!(failure.file, FileState::Absent);
    assert_eq!(failure.error.kind(), io::ErrorKind::StorageFull);
    assert_eq!(
        staged_in.as_deref(),
        Some(tmp.path().join("real").as_path())
    );
    assert!(
        names(&tmp.path().join("real")).is_empty(),
        "no target, no temp file"
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

/// Swarm review F1: through a chain of links, relative to each link, the
/// final target is created whole and every link is kept.
#[test]
fn a_write_through_a_chain_of_dangling_links_creates_the_final_target() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("real")).unwrap();
    symlink("real/b", tmp.path().join("a")).unwrap();
    symlink("c.txt", tmp.path().join("real/b")).unwrap();
    let written = replace_contents(&tmp.path().join("a"), b"hello").unwrap();
    assert_eq!(written, Written::Created);
    assert_eq!(
        std::fs::read(tmp.path().join("real/c.txt")).unwrap(),
        b"hello"
    );
    assert!(
        std::fs::symlink_metadata(tmp.path().join("a"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        std::fs::symlink_metadata(tmp.path().join("real/b"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(names(&tmp.path().join("real")), ["b", "c.txt"]);
}

/// Re-review: a file whose handle reports no links was removed meanwhile:
/// refused as such, never a panic, and nothing is written.
#[test]
fn a_file_removed_while_it_is_written_is_refused_not_a_panic() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "old").unwrap();
    let steps = Steps {
        links_of: |_| 0,
        ..Steps::real()
    };
    let failure = replace_contents_with(&target, b"new", steps).unwrap_err();
    assert_eq!(failure.file, FileState::Unchanged);
    assert_eq!(failure.error.kind(), io::ErrorKind::NotFound);
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    assert_eq!(names(tmp.path()), ["f.txt"]);
}

/// Re-review: a new file's temporary file that cannot be removed after a
/// failed write is named, not leaked silently.
#[test]
fn a_failed_create_names_a_temp_file_it_could_not_remove() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("new.txt");
    let steps = Steps {
        after_temp: Box::new(|_| Err(io::Error::from(io::ErrorKind::StorageFull))),
        remove: |_| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        ..Steps::real()
    };
    let failure = replace_contents_with(&target, b"x", steps).unwrap_err();
    assert_eq!(failure.file, FileState::Absent);
    let leftover = failure.leftover.expect("the temporary file is named");
    assert!(leftover.exists());
    assert_eq!(leftover.parent(), Some(tmp.path()));
    assert!(!target.exists());
}

/// Re-review: the same for a replacement that fails before its rename.
#[test]
fn a_failed_replacement_names_a_temp_file_it_could_not_remove() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "old").unwrap();
    let steps = Steps {
        after_temp: Box::new(|_| Err(io::Error::from(io::ErrorKind::StorageFull))),
        remove: |_| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        ..Steps::real()
    };
    let failure = replace_contents_with(&target, b"new", steps).unwrap_err();
    assert_eq!(failure.file, FileState::Unchanged);
    assert!(failure.leftover.is_some_and(|path| path.exists()));
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
}
