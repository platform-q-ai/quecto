use super::*;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(target_os = "linux")]
#[test]
fn a_new_file_holds_the_bytes_owner_only_behind_a_read_only_handle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("it's a \"file\" $HOME");
    let file = create_new(&path, b"#!/bin/sh\necho hi\n").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"#!/bin/sh\necho hi\n");
    assert_eq!(mode(&path), 0o600);
    let mut handle = &file;
    assert!(
        handle.write_all(b"x").is_err(),
        "the handle this process keeps is read-only"
    );
}

#[test]
fn empty_contents_make_an_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty");
    create_new(&path, b"").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"");
}

#[test]
fn a_taken_name_is_already_exists_and_is_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("taken");
    std::fs::write(&path, "theirs").unwrap();
    let error = create_new(&path, b"ours").unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
}

#[test]
fn a_dangling_symbolic_link_is_never_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let path = dir.path().join("link");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let error = create_new(&path, b"ours").unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(!target.exists(), "the link is refused, not followed");
}

#[test]
fn a_missing_directory_is_not_found_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing/file");
    let error = create_new(&path, b"x").unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(!path.exists());
}

#[test]
#[should_panic(expected = "names a file in a directory")]
fn a_path_without_a_file_name_is_refused() {
    let _ = create_new(Path::new("/"), b"x");
}

/// #2232 review round 2: the name is swapped for a symbolic link to a file
/// outside after the create and before the write. The bytes still land in
/// the file this call created, never through the link.
#[test]
fn a_symbolic_link_swapped_into_the_name_is_never_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("placed");
    let moved = dir.path().join("moved");
    let outside = dir.path().join("outside");
    std::fs::write(&outside, "outside").unwrap();
    let file = create_new_with(&path, b"ours", PLATFORM_WRITER, || {
        std::fs::rename(&path, &moved).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside");
    assert_eq!(std::fs::read(&moved).unwrap(), b"ours");
    let mut read_back = Vec::new();
    std::io::Read::read_to_end(&mut &file, &mut read_back).unwrap();
    assert_eq!(
        read_back, b"ours",
        "the returned handle is the written file"
    );
}

/// A FIFO swapped into the name is never opened: the write cannot hang on
/// a reader that never comes.
#[test]
fn a_fifo_swapped_into_the_name_is_never_opened() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("placed");
    let moved = dir.path().join("moved");
    let (done, finished) = std::sync::mpsc::channel();
    let (thread_path, thread_moved) = (path.clone(), moved.clone());
    std::thread::spawn(move || {
        let outcome = create_new_with(&thread_path, b"ours", PLATFORM_WRITER, || {
            std::fs::rename(&thread_path, &thread_moved).unwrap();
            let fifo = std::ffi::CString::new(thread_path.as_os_str().as_encoded_bytes()).unwrap();
            // SAFETY: `fifo` is a valid NUL-terminated path for the call's duration.
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        });
        let _ = done.send(outcome.map(|_| ()));
    });
    let outcome = finished
        .recv_timeout(Duration::from_secs(10))
        .expect("the write must not block on the swapped-in FIFO");
    outcome.unwrap();
    assert_eq!(std::fs::read(&moved).unwrap(), b"ours");
}

/// The child writer, the one whose reopen a file's mode can refuse (the
/// in-process writer already holds a writable descriptor).
#[cfg(target_os = "linux")]
const CHILD_WRITER: Writer = Writer::Child {
    reopen: "/dev/stdout",
};

/// The child writer cannot reopen a file its owner may not write (root
/// can, so the failure cannot be staged there).
#[cfg(target_os = "linux")]
fn writer_is_refused_by_mode() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() != 0 }
}

#[cfg(target_os = "linux")]
#[test]
fn a_failed_write_reports_the_writers_words_and_removes_the_file_it_created() {
    if !writer_is_refused_by_mode() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("placed");
    let error = create_new_with(&path, b"ours", CHILD_WRITER, || {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    })
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("the writer failed"), "{message}");
    assert!(message.contains("Permission denied"), "{message}");
    assert!(!path.exists(), "the file this call created is removed");
}

#[cfg(target_os = "linux")]
#[test]
fn a_failed_write_never_removes_someone_elses_file_in_the_name() {
    if !writer_is_refused_by_mode() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("placed");
    let moved = dir.path().join("moved");
    create_new_with(&path, b"ours", CHILD_WRITER, || {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
        std::fs::rename(&path, &moved).unwrap();
        std::fs::write(&path, "theirs").unwrap();
    })
    .unwrap_err();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
}

/// #2239 review: a failed write never removes a stranger's file that took
/// the name meanwhile. The write fails without any permission trick (the
/// writer writes elsewhere, the read-back catches it), so this holds as
/// root too.
#[test]
fn a_failed_write_never_removes_a_strangers_file_whoever_runs_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("placed");
    let moved = dir.path().join("moved");
    let elsewhere: &'static str = Box::leak(
        dir.path()
            .join("not-stdout")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str(),
    );
    let error = create_new_with(&path, b"ours", Writer::Child { reopen: elsewhere }, || {
        std::fs::rename(&path, &moved).unwrap();
        std::fs::write(&path, "theirs").unwrap();
    })
    .unwrap_err();
    assert!(error.to_string().contains("wrote elsewhere"), "{error}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
}

/// #2232 review round 3: a writer that exits 0 having written somewhere
/// else (a missing `/dev/stdout` recreated as a plain file in a writable
/// `/dev`) is caught by the read-back, never a silently empty file.
#[test]
fn a_writer_that_wrote_elsewhere_is_an_error_and_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("placed");
    let elsewhere: &'static str = Box::leak(
        dir.path()
            .join("not-stdout")
            .to_string_lossy()
            .into_owned()
            .into_boxed_str(),
    );
    let error =
        create_new_with(&path, b"ours", Writer::Child { reopen: elsewhere }, || {}).unwrap_err();
    assert!(error.to_string().contains("wrote elsewhere"), "{error}");
    assert!(!path.exists(), "the empty file is removed");
    assert_eq!(std::fs::read(elsewhere).unwrap(), b"ours");
}

/// The in-process writer (the platform's one off Linux) is exercised here
/// too: same bytes, same mode, same refusal of a taken name.
#[test]
fn the_in_process_writer_places_the_same_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("placed");
    create_new_with(&path, b"ours", Writer::InProcess, || {}).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"ours");
    assert_eq!(mode(&path), 0o600);
    let error = create_new_with(&path, b"again", Writer::InProcess, || {}).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
}

/// Set in the child run of the umask test: that process alone has the
/// restrictive umask.
const UMASK_CHILD: &str = "QUECTO_WRITER_FREE_FILE_UMASK_CHILD";

/// #2232 review round 3: under umask 0277 the create alone would leave mode
/// 0400 and the writer's reopen refused; the explicit `fchmod` keeps it
/// writable for the owner. The umask is process-wide, so the case runs in a
/// child process of this test binary.
#[test]
fn a_restrictive_umask_does_not_refuse_the_writer() {
    if std::env::var_os(UMASK_CHILD).is_some() {
        // The directory first: under the umask it would be unwritable.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("placed");
        // SAFETY: umask only swaps this process's file-creation mask.
        unsafe { libc::umask(0o277) };
        create_new(&path, b"ours").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"ours");
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "infrastructure::processes::writer_free_file::tests::a_restrictive_umask_does_not_refuse_the_writer",
            "--test-threads=1",
        ])
        .env(UMASK_CHILD, "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "the umask 0277 run failed: {stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// #2232 review round 4: the in-process writer writes positioned, so the
/// handle it returns reads the file from its start, as the child writer's
/// untouched handle does.
#[test]
fn the_in_process_writer_returns_a_handle_that_reads_from_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("placed");
    let file = create_new_with(&path, b"ours", Writer::InProcess, || {}).unwrap();
    let mut read = Vec::new();
    std::io::Read::read_to_end(&mut &file, &mut read).unwrap();
    assert_eq!(read, b"ours");
}
