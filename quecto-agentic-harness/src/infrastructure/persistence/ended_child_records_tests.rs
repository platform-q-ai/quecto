use super::*;
use crate::application::sessions::ports::SessionStore;
use crate::domain::message::Message;
use crate::domain::session::Session;

fn child() -> SessionIdentity {
    SessionIdentity::named_cli("child").unwrap()
}

async fn save(store: &FileSessionStore, texts: &[&str]) {
    let mut session = Session::new(child());
    session.messages = texts.iter().map(|text| Message::user(*text)).collect();
    store.save(&session).await.unwrap();
}

fn records(base: &std::path::Path) -> (FileEndedChildRecords, Arc<FileSessionStore>) {
    let store = Arc::new(crate::composition::sessions::build_file_session_store(base));
    (FileEndedChildRecords::new(base, store.clone()), store)
}

fn texts(transcript: &EndedTranscript) -> Vec<&str> {
    transcript
        .messages
        .iter()
        .map(|m| m.content.as_str())
        .collect()
}

#[tokio::test]
async fn an_unchanged_transcript_is_shared_not_read_again() {
    let base = tempfile::tempdir().unwrap();
    let (records, store) = records(base.path());
    save(&store, &["one"]).await;
    let first = records.transcript(&child()).await.unwrap().unwrap();
    let again = records.transcript(&child()).await.unwrap().unwrap();
    assert!(
        Arc::ptr_eq(&first.messages, &again.messages),
        "shared, not re-read"
    );
}

#[tokio::test]
async fn a_changed_transcript_is_read_afresh() {
    let base = tempfile::tempdir().unwrap();
    let (records, store) = records(base.path());
    save(&store, &["one"]).await;
    let first = records.transcript(&child()).await.unwrap().unwrap();
    assert_eq!(texts(&first), ["one"]);
    save(&store, &["one", "two"]).await;
    let changed = records.transcript(&child()).await.unwrap().unwrap();
    assert_eq!(texts(&changed), ["one", "two"]);
}

#[tokio::test]
async fn a_read_that_hit_a_torn_record_is_not_kept() {
    let base = tempfile::tempdir().unwrap();
    let (records, store) = records(base.path());
    save(&store, &["one"]).await;
    let path = base.path().join("sessions/cli_child.json");
    let mut data = std::fs::read(&path).unwrap();
    data.extend_from_slice(br#"{"type":"append","messages":[{"role""#);
    std::fs::write(&path, data).unwrap();
    let first = records.transcript(&child()).await.unwrap().unwrap();
    let again = records.transcript(&child()).await.unwrap().unwrap();
    assert!(!Arc::ptr_eq(&first.messages, &again.messages), "read again");
}

#[tokio::test]
async fn a_transcript_over_the_bound_gives_its_newest_part_marked_as_such() {
    let base = tempfile::tempdir().unwrap();
    let store = Arc::new(crate::composition::sessions::build_file_session_store(
        base.path(),
    ));
    let records = FileEndedChildRecords::with_bound(base.path(), store.clone(), 600);
    let all: Vec<String> = (0..20)
        .map(|i| format!("m{i:02}-{}", "y".repeat(80)))
        .collect();
    for n in 1..=all.len() {
        let refs: Vec<&str> = all[..n].iter().map(String::as_str).collect();
        save(&store, &refs).await;
    }
    let transcript = records.transcript(&child()).await.unwrap().unwrap();
    assert!(transcript.older_omitted);
    assert_eq!(texts(&transcript).last(), Some(&all[19].as_str()));
    assert!(transcript.messages.len() < all.len());
}

/// #2192 review: two children read in turn do not evict each other — each
/// unchanged transcript is shared, not read again — while the cache stays
/// bounded, the least recently read given up first.
#[tokio::test]
async fn interleaved_readers_keep_each_childs_transcript() {
    let base = tempfile::tempdir().unwrap();
    let (records, store) = records(base.path());
    let children: Vec<SessionIdentity> = (0..=MAX_KEPT_TRANSCRIPTS)
        .map(|n| SessionIdentity::named_cli(&format!("child-{n}")).unwrap())
        .collect();
    for child in &children {
        let mut session = Session::new(child.clone());
        session.messages = vec![Message::user("work")];
        store.save(&session).await.unwrap();
    }
    let first = records.transcript(&children[0]).await.unwrap().unwrap();
    let second = records.transcript(&children[1]).await.unwrap().unwrap();
    let first_again = records.transcript(&children[0]).await.unwrap().unwrap();
    let second_again = records.transcript(&children[1]).await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&first.messages, &first_again.messages));
    assert!(Arc::ptr_eq(&second.messages, &second_again.messages));
    for child in &children[2..] {
        records.transcript(child).await.unwrap().unwrap();
    }
    let kept = records.kept_children();
    assert_eq!(kept.len(), MAX_KEPT_TRANSCRIPTS);
    assert!(!kept.contains(&children[0]), "the least recently read went");
    assert!(kept.contains(&children[1]));
}

/// The cache never holds more than one transcript's bound of files.
#[tokio::test]
async fn the_cache_holds_at_most_one_bound_of_files() {
    let base = tempfile::tempdir().unwrap();
    let store = Arc::new(crate::composition::sessions::build_file_session_store(
        base.path(),
    ));
    let records = FileEndedChildRecords::with_bound(base.path(), store.clone(), 4096);
    let children: Vec<SessionIdentity> = (0..3)
        .map(|n| SessionIdentity::named_cli(&format!("big-{n}")).unwrap())
        .collect();
    for child in &children {
        let mut session = Session::new(child.clone());
        session.messages = vec![Message::user("x".repeat(1500))];
        store.save(&session).await.unwrap();
        records.transcript(child).await.unwrap().unwrap();
    }
    assert_eq!(records.kept_children(), children[1..].to_vec());
}

#[tokio::test]
async fn whether_a_transcript_is_there_is_asked_without_reading_it() {
    let base = tempfile::tempdir().unwrap();
    let (records, store) = records(base.path());
    assert!(!records.has_transcript(&child()).await);
    save(&store, &["one"]).await;
    assert!(records.has_transcript(&child()).await);
    assert!(records.kept_children().is_empty(), "nothing was read");
    let path = base.path().join("sessions/cli_child.json");
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(base.path(), &path).unwrap();
    assert!(!records.has_transcript(&child()).await, "a link is not one");
}
