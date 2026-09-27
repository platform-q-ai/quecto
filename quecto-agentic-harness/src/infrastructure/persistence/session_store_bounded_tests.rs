use super::*;
use crate::application::sessions::ports::SessionStore;
use crate::domain::session::Session;

fn store(base: &std::path::Path) -> FileSessionStore {
    crate::composition::sessions::build_file_session_store(base)
}

fn child() -> SessionIdentity {
    SessionIdentity::named_cli("child").unwrap()
}

async fn save(base: &std::path::Path, texts: &[&str]) -> std::path::PathBuf {
    let mut session = Session::new(child());
    session.messages = texts.iter().map(|text| Message::user(*text)).collect();
    store(base).save(&session).await.unwrap();
    base.join("sessions/cli_child.json")
}

fn contents(read: &UntrustedRead) -> Vec<String> {
    match read {
        UntrustedRead::Read { messages, .. } => {
            messages.iter().map(|m| m.content.clone()).collect()
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_session_within_the_bound_is_read_whole() {
    let base = tempfile::tempdir().unwrap();
    save(base.path(), &["before the crash"]).await;
    let read = store(base.path())
        .load_untrusted(&child(), 1 << 20, None)
        .await
        .unwrap();
    assert_eq!(contents(&read), ["before the crash"]);
    assert!(matches!(
        read,
        UntrustedRead::Read {
            complete: true,
            older_omitted: false,
            ..
        }
    ));
    let other = SessionIdentity::named_cli("other").unwrap();
    assert!(matches!(
        store(base.path())
            .load_untrusted(&other, 1 << 20, None)
            .await
            .unwrap(),
        UntrustedRead::Missing
    ));
}

#[tokio::test]
async fn an_unchanged_file_is_not_read_again_and_a_changed_one_is() {
    let base = tempfile::tempdir().unwrap();
    save(base.path(), &["one"]).await;
    let UntrustedRead::Read { stamp, .. } = store(base.path())
        .load_untrusted(&child(), 1 << 20, None)
        .await
        .unwrap()
    else {
        panic!("read")
    };
    assert!(matches!(
        store(base.path())
            .load_untrusted(&child(), 1 << 20, Some(stamp))
            .await
            .unwrap(),
        UntrustedRead::Unchanged
    ));
    save(base.path(), &["one", "two"]).await;
    let read = store(base.path())
        .load_untrusted(&child(), 1 << 20, Some(stamp))
        .await
        .unwrap();
    assert_eq!(contents(&read), ["one", "two"]);
}

#[tokio::test]
async fn a_file_over_the_bound_gives_its_newest_messages() {
    let base = tempfile::tempdir().unwrap();
    // Appended one save at a time, so the file is a snapshot and appends.
    let texts: Vec<String> = (0..40)
        .map(|i| format!("m{i:02}-{}", "x".repeat(100)))
        .collect();
    // One store saves every turn, so it appends onto the file it wrote.
    let appender = store(base.path());
    let mut session = Session::new(child());
    for text in &texts {
        session.messages.push(Message::user(text.as_str()));
        appender.save(&session).await.unwrap();
    }
    let read = store(base.path())
        .load_untrusted(&child(), 1500, None)
        .await
        .unwrap();
    let UntrustedRead::Read {
        older_omitted,
        complete,
        ..
    } = &read
    else {
        panic!("{read:?}")
    };
    assert!(*older_omitted && *complete);
    let got = contents(&read);
    assert!(!got.is_empty() && got.len() < texts.len(), "{}", got.len());
    assert_eq!(got.last(), texts.last(), "the newest message is there");
}

#[tokio::test]
async fn a_torn_trailing_record_is_reported_incomplete() {
    let base = tempfile::tempdir().unwrap();
    let path = save(base.path(), &["one"]).await;
    let mut data = std::fs::read(&path).unwrap();
    data.extend_from_slice(br#"{"type":"append","messages":[{"role":"user","con"#);
    std::fs::write(&path, data).unwrap();
    let read = store(base.path())
        .load_untrusted(&child(), 1 << 20, None)
        .await
        .unwrap();
    assert_eq!(contents(&read), ["one"]);
    assert!(matches!(
        read,
        UntrustedRead::Read {
            complete: false,
            ..
        }
    ));
}

#[tokio::test]
async fn a_link_or_a_fifo_is_neither_followed_nor_waited_on() {
    let base = tempfile::tempdir().unwrap();
    let path = save(base.path(), &["x"]).await;
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink("/dev/zero", &path).unwrap();
    assert!(
        store(base.path())
            .load_untrusted(&child(), 1 << 20, None)
            .await
            .is_err()
    );
    std::fs::remove_file(&path).unwrap();
    let fifo = std::ffi::CString::new(path.into_os_string().into_encoded_bytes()).unwrap();
    // SAFETY: `fifo` is NUL-terminated.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        store(base.path()).load_untrusted(&child(), 1 << 20, None),
    )
    .await
    .expect("a FIFO is not waited on");
    let error = read.unwrap_err().to_string();
    assert!(error.contains("not a regular file"), "{error}");
}

#[tokio::test]
async fn a_link_to_another_sessions_file_is_not_read_as_the_childs() {
    let base = tempfile::tempdir().unwrap();
    let path = save(base.path(), &["x"]).await;
    let elsewhere = base.path().join("elsewhere.json");
    std::fs::rename(&path, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    let error = store(base.path())
        .load_untrusted(&child(), 1 << 20, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("a link is never followed"), "{error}");
}

/// L-4 (#2192 review): a same-length rewrite in place whose modification
/// time is put back still changes the file's status-change time, which the
/// stamp carries, so the cached transcript is not served stale.
#[tokio::test]
async fn a_same_length_rewrite_with_its_mtime_restored_is_read_again() {
    let base = tempfile::tempdir().unwrap();
    let path = save(base.path(), &["aaaa"]).await;
    let UntrustedRead::Read { stamp, .. } = store(base.path())
        .load_untrusted(&child(), 1 << 20, None)
        .await
        .unwrap()
    else {
        panic!("read")
    };
    let before = std::fs::metadata(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    // A later ctime needs a later clock tick than the save's.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, text.replace("aaaa", "bbbb")).unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(before.modified().unwrap()).unwrap();
    drop(file);
    let after = std::fs::metadata(&path).unwrap();
    assert_eq!(
        (after.len(), after.modified().unwrap()),
        (before.len(), before.modified().unwrap()),
        "length and mtime are as they were"
    );
    let read = store(base.path())
        .load_untrusted(&child(), 1 << 20, Some(stamp))
        .await
        .unwrap();
    assert_eq!(contents(&read), ["bbbb"]);
}

fn append(start_index: Option<usize>, text: &str) -> String {
    let record = super::super::SessionRecord::Append {
        start_index,
        messages: vec![super::super::super::message_to_record(&Message::user(text))],
        workflow_run: None,
        workflow_run_cleared: false,
        subagent_roster: None,
    };
    serde_json::to_string(&record).unwrap()
}

fn texts(messages: &[Message]) -> Vec<&str> {
    messages.iter().map(|m| m.content.as_str()).collect()
}

/// Info (#2192 review): the newest part is held to the same order the full
/// parse holds a file to — an append out of place ends the read.
#[tokio::test]
async fn a_tail_read_stops_at_an_append_out_of_place() {
    let base = tempfile::tempdir().unwrap();
    let path = save(base.path(), &["one", "two"]).await;
    let snapshot = std::fs::read_to_string(&path).unwrap();
    let snapshot = snapshot.lines().next().unwrap();
    let data = [
        snapshot.to_string(),
        append(Some(2), "three"),
        append(Some(7), "forged"),
        append(Some(3), "four"),
    ]
    .join("\n");
    assert_eq!(texts(&tail_messages(&data)), ["one", "two", "three"]);
    // No snapshot in the tail: the first indexed append sets where it is.
    let data = [
        append(Some(5), "five"),
        append(Some(6), "six"),
        append(Some(9), "forged"),
    ]
    .join("\n");
    assert_eq!(texts(&tail_messages(&data)), ["five", "six"]);
    // Unindexed appends (older files) are taken as the full parse takes them.
    let data = [
        append(None, "a"),
        append(Some(10), "b"),
        append(Some(11), "c"),
    ]
    .join("\n");
    assert_eq!(texts(&tail_messages(&data)), ["a", "b", "c"]);
    let data = [append(None, "a"), append(None, "b"), append(Some(1), "c")].join("\n");
    assert_eq!(
        texts(&tail_messages(&data)),
        ["a", "b"],
        "index 1 is not after two"
    );
}
