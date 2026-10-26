//! A follower's token file (epic #2705 B11 review): read when it is this
//! user's alone; a FIFO, a link or a readable file is refused at once and
//! unquoted.
use std::os::unix::fs::PermissionsExt;

use super::*;

const TOKEN: &str = "A-SECRET-TOKEN-OF-SIXTEEN-OR-MORE";

fn written(dir: &Path, mode: u32) -> std::path::PathBuf {
    let path = dir.join("primary.token");
    std::fs::write(&path, format!("{TOKEN}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

#[test]
fn a_private_file_gives_its_token() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        read_token_file(&written(dir.path(), 0o600)),
        Ok(TOKEN.into())
    );
    assert_eq!(
        read_token_file(&written(dir.path(), 0o400)),
        Ok(TOKEN.into())
    );
}

#[test]
fn a_file_others_may_read_is_refused_unquoted() {
    let dir = tempfile::tempdir().unwrap();
    let refused = read_token_file(&written(dir.path(), 0o644)).unwrap_err();
    assert!(refused.contains("no one else may read"), "{refused}");
    assert!(!refused.contains(TOKEN), "{refused}");
}

#[test]
fn a_fifo_is_refused_at_once_never_waited_on() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("primary.token");
    let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: a NUL-ended path; mkfifo only makes the node.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let (sent, read) = std::sync::mpsc::channel();
    std::thread::spawn(move || sent.send(read_token_file(&fifo)));
    let answer = read.recv_timeout(std::time::Duration::from_secs(5));
    let refused = answer.expect("a FIFO never hangs the read").unwrap_err();
    assert!(refused.contains("regular file"), "{refused}");
}

/// B11 review 2: the diverged mark is kept the same way: written private,
/// read back; a link or FIFO in its place is never followed.
#[test]
fn a_private_mark_is_written_and_read_and_never_through_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let mark = dir.path().join("board.sqlite3.diverged");
    assert_eq!(read_private(&mark, "the mark"), Ok(None));
    write_private(&mark, "the mark", "forked\n").unwrap();
    assert_eq!(read_private(&mark, "the mark"), Ok(Some("forked\n".into())));
    let mode = std::fs::metadata(&mark).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let elsewhere = dir.path().join("elsewhere");
    std::fs::write(&elsewhere, "x").unwrap();
    let link = dir.path().join("linked.diverged");
    std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
    assert!(write_private(&link, "the mark", "forked").is_err());
    assert_eq!(
        std::fs::read_to_string(&elsewhere).unwrap(),
        "x",
        "never followed"
    );
    assert!(read_private(&link, "the mark").is_err());
}

#[test]
fn a_link_to_a_private_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let target = written(dir.path(), 0o600);
    let link = dir.path().join("link.token");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(read_token_file(&link).unwrap_err().contains("regular file"));
}
