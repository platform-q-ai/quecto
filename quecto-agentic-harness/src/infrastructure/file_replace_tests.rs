// #2243: a file's contents are replaced whole or not at all; a failed write
// never leaves it empty or cut short.

use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use tempfile::TempDir;

/// The names in `dir`, sorted: no temporary file may be left behind.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The real steps, with `after` run once the temporary file exists.
fn with_after_temp<'a>(after: impl FnMut(&Path) -> std::io::Result<()> + 'a) -> Steps<'a> {
    Steps {
        after_temp: Box::new(after),
        ..Steps::real()
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

/// Permissions do not stop root: tests of refusals skip then.
fn is_root() -> bool {
    let probe = TempDir::new().unwrap();
    let locked = probe.path().join("locked");
    std::fs::write(&locked, "x").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o400)).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&locked)
        .is_ok()
}

#[test]
fn the_file_is_replaced_whole_by_a_rename_and_no_temp_file_is_left() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "old text\n").unwrap();
    let inode = std::fs::metadata(&target).unwrap().ino();
    let written = replace_contents(&target, b"new text\n").unwrap();
    assert_eq!(written, Written::Replaced);
    assert_eq!(std::fs::read(&target).unwrap(), b"new text\n");
    assert_ne!(
        std::fs::metadata(&target).unwrap().ino(),
        inode,
        "renamed in"
    );
    assert_eq!(names(tmp.path()), ["f.txt"]);
}

#[test]
fn a_failure_after_the_temp_file_is_created_leaves_the_original_and_no_temp_file() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "precious\n").unwrap();
    let mut seen = None;
    let failure = replace_contents_with(
        &target,
        b"new",
        with_after_temp(|temp| {
            assert!(temp.exists(), "the hook runs once the temp file exists");
            seen = Some(temp.to_path_buf());
            Err(std::io::Error::other("disk full (injected)"))
        }),
    )
    .unwrap_err();
    assert_eq!(failure.file, FileState::Unchanged);
    assert_eq!(failure.error.to_string(), "disk full (injected)");
    assert_eq!(std::fs::read(&target).unwrap(), b"precious\n");
    assert_eq!(names(tmp.path()), ["f.txt"]);
    let temp = seen.expect("the hook ran");
    assert_eq!(temp.parent(), Some(tmp.path()), "same directory");
    let name = temp.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with(".f.txt.") && name.ends_with(".tmp"),
        "{name}"
    );
}

#[test]
fn temp_names_are_unpredictable() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "x").unwrap();
    let mut temps = Vec::new();
    for _ in 0..2 {
        let _ = replace_contents_with(
            &target,
            b"y",
            with_after_temp(|temp| {
                temps.push(temp.to_path_buf());
                Err(std::io::Error::other("stop"))
            }),
        );
    }
    assert_eq!(temps.len(), 2);
    assert_ne!(temps[0], temps[1]);
}

#[test]
fn the_mode_is_kept_whatever_the_umask() {
    let tmp = TempDir::new().unwrap();
    for wanted in [0o640, 0o666, 0o755, 0o600] {
        let target = tmp.path().join(format!("f{wanted:o}"));
        std::fs::write(&target, "old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(wanted)).unwrap();
        let mut temp_mode = None;
        replace_contents_with(
            &target,
            b"new",
            with_after_temp(|temp| {
                temp_mode = Some(mode(temp));
                Ok(())
            }),
        )
        .unwrap();
        assert_eq!(
            temp_mode,
            Some(wanted),
            "the temp file has it from the start"
        );
        assert_eq!(mode(&target), wanted);
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }
}

#[test]
fn the_owner_is_kept() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "old").unwrap();
    let before = std::fs::metadata(&target).unwrap();
    replace_contents(&target, b"new").unwrap();
    let after = std::fs::metadata(&target).unwrap();
    assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
}

#[test]
fn a_symbolic_link_is_kept_and_its_target_is_written() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("real")).unwrap();
    std::fs::create_dir(tmp.path().join("links")).unwrap();
    let target = tmp.path().join("real/f.txt");
    std::fs::write(&target, "old").unwrap();
    let link = tmp.path().join("links/f.txt");
    symlink("../real/f.txt", &link).unwrap();
    let chained = tmp.path().join("links/g.txt");
    symlink("f.txt", &chained).unwrap();

    assert_eq!(replace_contents(&link, b"one").unwrap(), Written::Replaced);
    assert_eq!(std::fs::read(&target).unwrap(), b"one");
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        Path::new("../real/f.txt")
    );

    assert_eq!(
        replace_contents(&chained, b"two").unwrap(),
        Written::Replaced
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"two");
    assert!(
        std::fs::symlink_metadata(&chained)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(names(&tmp.path().join("real")), ["f.txt"]);
    assert_eq!(names(&tmp.path().join("links")), ["f.txt", "g.txt"]);
}

#[test]
fn a_hard_linked_file_is_written_in_place_to_keep_its_links() {
    let tmp = TempDir::new().unwrap();
    let a = tmp.path().join("a.txt");
    let b = tmp.path().join("b.txt");
    std::fs::write(&a, "a longer original text\n").unwrap();
    std::fs::hard_link(&a, &b).unwrap();
    let inode = std::fs::metadata(&a).unwrap().ino();
    let written = replace_contents(&a, b"short\n").unwrap();
    assert_eq!(written, Written::InPlace(InPlace::HardLinks(2)));
    assert_eq!(
        std::fs::read(&a).unwrap(),
        b"short\n",
        "cut to the new length"
    );
    assert_eq!(std::fs::read(&b).unwrap(), b"short\n", "the link sees it");
    assert_eq!(std::fs::metadata(&a).unwrap().ino(), inode);
    assert_eq!(std::fs::metadata(&a).unwrap().nlink(), 2);
    assert_eq!(names(tmp.path()), ["a.txt", "b.txt"]);
}

#[test]
fn a_new_file_is_created_and_a_dangling_link_is_written_through() {
    let tmp = TempDir::new().unwrap();
    let fresh = tmp.path().join("new.txt");
    assert_eq!(
        replace_contents(&fresh, b"hello").unwrap(),
        Written::Created
    );
    assert_eq!(std::fs::read(&fresh).unwrap(), b"hello");

    let link = tmp.path().join("link.txt");
    symlink("made.txt", &link).unwrap();
    assert_eq!(
        replace_contents(&link, b"through").unwrap(),
        Written::Created
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read(tmp.path().join("made.txt")).unwrap(),
        b"through"
    );
    assert_eq!(names(tmp.path()), ["link.txt", "made.txt", "new.txt"]);
}

#[test]
fn a_directory_is_refused_and_left_alone() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("d");
    std::fs::create_dir(&dir).unwrap();
    let failure = replace_contents(&dir, b"x").unwrap_err();
    assert_eq!(failure.file, FileState::Unchanged);
    assert_eq!(failure.error.kind(), std::io::ErrorKind::IsADirectory);
    assert!(dir.is_dir());
    assert_eq!(names(tmp.path()), ["d"]);
}

#[test]
fn a_read_only_file_is_refused_untouched() {
    if is_root() {
        return; // Running as root: permissions do not stop the write.
    }
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o444)).unwrap();
    let failure = replace_contents(&target, b"new").unwrap_err();
    assert_eq!(failure.error.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(failure.file, FileState::Unchanged);
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    assert_eq!(names(tmp.path()), ["f.txt"]);
}

#[test]
fn a_writable_file_in_a_locked_directory_is_written_in_place() {
    if is_root() {
        return; // Running as root: permissions do not stop the create.
    }
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("locked");
    std::fs::create_dir(&dir).unwrap();
    let target = dir.join("f.txt");
    std::fs::write(&target, "old").unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let written = replace_contents(&target, b"new");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        written.unwrap(),
        Written::InPlace(InPlace::DirectoryNotWritable)
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    assert_eq!(names(&dir), ["f.txt"]);
}

#[test]
fn an_in_place_write_that_fails_may_have_cut_the_file_short() {
    let tmp = TempDir::new().unwrap();
    let a = tmp.path().join("a.txt");
    std::fs::write(&a, "old").unwrap();
    std::fs::hard_link(&a, tmp.path().join("b.txt")).unwrap();
    let file = std::fs::OpenOptions::new().read(true).open(&a).unwrap();
    // A read-only handle: the write fails as a write error would.
    let failure = write_in_place(&file, b"new").unwrap_err();
    assert_eq!(failure.file, FileState::MayBeCutShort);
}

/// A new file gets the mode any new file gets (0666 less the umask), as a
/// plain write would give it.
#[test]
fn a_new_file_gets_the_default_mode() {
    let tmp = TempDir::new().unwrap();
    let probe = tmp.path().join("probe");
    std::fs::write(&probe, "x").unwrap();
    let fresh = tmp.path().join("fresh");
    replace_contents(&fresh, b"x").unwrap();
    assert_eq!(mode(&fresh), mode(&probe));
}

/// Another group the writer is in is given back to the new file. Skipped
/// where the writer is in no other group.
#[test]
fn another_group_is_given_back() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("f.txt");
    std::fs::write(&target, "old").unwrap();
    let own = std::fs::metadata(&target).unwrap().gid();
    // SAFETY: getgroups with a zero size only counts the groups.
    let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    let mut groups = vec![0 as libc::gid_t; usize::try_from(count).unwrap_or(0)];
    // SAFETY: `groups` holds `count` entries, the size passed.
    let got = unsafe { libc::getgroups(count, groups.as_mut_ptr()) };
    groups.truncate(usize::try_from(got).unwrap_or(0));
    let Some(other) = groups.into_iter().find(|group| *group != own) else {
        return;
    };
    let file = std::fs::File::open(&target).unwrap();
    std::os::unix::fs::fchown(&file, None, Some(other)).unwrap();
    replace_contents(&target, b"new").unwrap();
    assert_eq!(std::fs::metadata(&target).unwrap().gid(), other);
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
}
