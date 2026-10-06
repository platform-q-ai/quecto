//! A record torn by a crash mid-append (#2218, the review's `torn.py`):
//! the next save of every kind compacts over it instead of gluing its
//! record onto the unterminated line, so nothing saved later is lost and
//! no ordinal is handed out twice after a restart.
use super::*;
use crate::application::sessions::ports::SessionStore;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use tempfile::TempDir;

const TORN: &[u8] = br#"{"type":"append","start_index":2,"messages":[{"role":"us"#;

fn id() -> SessionIdentity {
    SessionIdentity::from_persisted_key("cli:torn")
}

pub(super) fn transcript(len: usize) -> Vec<Message> {
    let mut messages: Vec<Message> = (0..len)
        .map(|i| match i % 2 {
            0 => Message::user(format!("task {i}")),
            _ => Message::assistant(format!("report {i}"), vec![]),
        })
        .collect();
    crate::domain::sessions::entities::session::assign_missing_ordinals(&mut messages);
    messages
}

/// A store holding a two-message transcript whose file then gets a torn
/// trailing record, as a process killed mid-append leaves it.
async fn torn_store() -> (TempDir, FileSessionStore) {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    store
        .save_clean_delta(&id(), &transcript(2), 0, None)
        .await
        .unwrap();
    let path = store.session_path(&id());
    let mut data = std::fs::read(&path).unwrap();
    assert!(data.ends_with(b"\n"), "a finished save is terminated");
    data.extend_from_slice(TORN);
    std::fs::write(&path, data).unwrap();
    (tmp, store)
}

async fn assert_reloads_whole(store: &FileSessionStore, len: usize) {
    let path = store.session_path(&id());
    assert!(std::fs::read(&path).unwrap().ends_with(b"\n"));
    let fresh = FileSessionStore::new(FlatSessionLayout::new(
        path.parent().unwrap().parent().unwrap(),
    ));
    let loaded = fresh.load(&id()).await.unwrap().expect("saved");
    assert_eq!(
        loaded
            .messages
            .iter()
            .map(|m| m.content.clone())
            .collect::<Vec<_>>(),
        transcript(len)
            .iter()
            .map(|m| m.content.clone())
            .collect::<Vec<_>>(),
        "a restart loads every message saved after the torn record"
    );
    let ordinals: Vec<_> = loaded.messages.iter().map(|m| m.ordinal).collect();
    assert_eq!(ordinals, (1..=len as u64).map(Some).collect::<Vec<_>>());
}

#[tokio::test]
async fn a_clean_delta_after_a_torn_record_compacts() {
    let (_tmp, store) = torn_store().await;
    store
        .save_clean_delta(&id(), &transcript(4), 2, None)
        .await
        .unwrap();
    store
        .save_clean_delta(&id(), &transcript(6), 4, None)
        .await
        .unwrap();
    assert_reloads_whole(&store, 6).await;
}

#[tokio::test]
async fn a_full_save_after_a_torn_record_compacts() {
    let (_tmp, store) = torn_store().await;
    let mut session = Session::new(id());
    session.messages = transcript(4);
    store.save(&session).await.unwrap();
    session.messages = transcript(6);
    store.save(&session).await.unwrap();
    assert_reloads_whole(&store, 6).await;
}

#[tokio::test]
async fn a_verified_delta_after_a_torn_record_compacts() {
    let (_tmp, store) = torn_store().await;
    store
        .save_delta(&id(), &transcript(4), 2, None)
        .await
        .unwrap();
    store
        .save_delta(&id(), &transcript(6), 4, None)
        .await
        .unwrap();
    assert_reloads_whole(&store, 6).await;
}

#[tokio::test]
async fn an_intact_file_is_still_appended_to() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    store
        .save_clean_delta(&id(), &transcript(2), 0, None)
        .await
        .unwrap();
    store
        .save_clean_delta(&id(), &transcript(4), 2, None)
        .await
        .unwrap();
    let data = std::fs::read_to_string(store.session_path(&id())).unwrap();
    assert_eq!(data.lines().count(), 2, "snapshot then one append: {data}");
    assert_reloads_whole(&store, 4).await;
}

#[tokio::test]
async fn a_dropped_record_ending_in_a_newline_still_compacts() {
    let (_tmp, store) = torn_store().await;
    let path = store.session_path(&id());
    let mut data = std::fs::read(&path).unwrap();
    data.push(b'\n');
    std::fs::write(&path, data).unwrap();
    let mut session = Session::new(id());
    session.messages = transcript(4);
    store.save(&session).await.unwrap();
    store
        .save_delta(&id(), &transcript(6), 4, None)
        .await
        .unwrap();
    assert_reloads_whole(&store, 6).await;
}

/// A crash can cut an append just before its newline: the record parses,
/// but a later append would be glued onto its line.
#[tokio::test]
async fn a_record_missing_only_its_newline_is_never_appended_to() {
    for kind in ["clean", "verified", "full"] {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
        store
            .save_clean_delta(&id(), &transcript(2), 0, None)
            .await
            .unwrap();
        store
            .save_clean_delta(&id(), &transcript(4), 2, None)
            .await
            .unwrap();
        let path = store.session_path(&id());
        let mut data = std::fs::read(&path).unwrap();
        assert_eq!(data.pop(), Some(b'\n'));
        std::fs::write(&path, data).unwrap();
        match kind {
            "clean" => store.save_clean_delta(&id(), &transcript(6), 4, None).await,
            "verified" => store.save_delta(&id(), &transcript(6), 4, None).await,
            _ => {
                let mut session = Session::new(id());
                session.messages = transcript(6);
                store.save(&session).await
            }
        }
        .unwrap();
        assert_reloads_whole(&store, 6).await;
    }
}

/// A save whose record landed (#2218 review 3) but which reported failure (sync_data
/// EIO after write_all, or dir fsync after rename) is retried as a clean
/// delta from the unchanged watermark. A failed write makes the store forget
/// the file was intact (here: a store that never saw it), so the retry
/// verifies the file, finds it longer than the watermark, and compacts.
#[tokio::test]
async fn a_retry_after_a_landed_but_failed_append_compacts() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    store
        .save_clean_delta(&id(), &transcript(2), 0, None)
        .await
        .unwrap();
    // append landed, caller saw Err: watermark stays 2
    store
        .save_clean_delta(&id(), &transcript(4), 2, None)
        .await
        .unwrap();
    store.release(&id());
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    // retry with next turn, same watermark
    store
        .save_clean_delta(&id(), &transcript(6), 2, None)
        .await
        .unwrap();
    store
        .save_clean_delta(&id(), &transcript(8), 6, None)
        .await
        .unwrap();
    let loaded = store.load(&id()).await.unwrap().unwrap();
    assert_eq!(loaded.messages.len(), 8);
}

/// A dropped (unparseable) record that ends with a newline, loaded after a
/// restart, then a clean delta (the per-turn path): the load saw the file
/// was not intact, so the delta compacts (#2218 review 3).
#[tokio::test]
async fn a_clean_delta_after_a_loaded_dropped_record_compacts() {
    let (_tmp, store) = torn_store().await;
    let path = store.session_path(&id());
    let mut data = std::fs::read(&path).unwrap();
    data.push(b'\n');
    std::fs::write(&path, data).unwrap();
    // restart: watermark = loaded len
    let loaded = store.load(&id()).await.unwrap().unwrap();
    let n = loaded.messages.len();
    store
        .save_clean_delta(&id(), &transcript(4), n, None)
        .await
        .unwrap();
    let loaded = store.load(&id()).await.unwrap().unwrap();
    assert_eq!(loaded.messages.len(), 4);
}

/// Every crash boundary of an append, then a restart and a clean delta; a
/// temporary file a crash left before its rename is ignored (#2218 review 3).
#[tokio::test]
async fn every_crash_boundary_of_an_append_recovers() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    store
        .save_clean_delta(&id(), &transcript(2), 0, None)
        .await
        .unwrap();
    store
        .save_clean_delta(&id(), &transcript(4), 2, None)
        .await
        .unwrap();
    let path = store.session_path(&id());
    let full = std::fs::read(&path).unwrap();
    let first_nl = full.iter().position(|b| *b == b'\n').unwrap() + 1;
    // every truncation point of the append record
    for cut in first_nl..=full.len() {
        std::fs::write(&path, &full[..cut]).unwrap();
        let loaded = store.load(&id()).await.unwrap().unwrap();
        let len = loaded.messages.len();
        assert!(
            len == 2 || (len == 4 && cut >= full.len() - 1),
            "cut {cut}: {len}"
        );
        let ords: Vec<_> = loaded.messages.iter().map(|m| m.ordinal).collect();
        assert_eq!(ords, (1..=len as u64).map(Some).collect::<Vec<_>>());
        // next save after restart from loaded len
        store
            .save_clean_delta(&id(), &transcript(6), len, None)
            .await
            .unwrap();
        let again = store.load(&id()).await.unwrap().unwrap();
        assert_eq!(
            again.messages.len(),
            6,
            "cut {cut}: after-restart save lost"
        );
        let ords: Vec<_> = again.messages.iter().map(|m| m.ordinal).collect();
        assert_eq!(ords, (1..=6u64).map(Some).collect::<Vec<_>>(), "cut {cut}");
    }
    // leftover temp file (crash before rename) is ignored by load
    std::fs::write(path.with_extension("tmp"), b"{garbage").unwrap();
    assert_eq!(store.load(&id()).await.unwrap().unwrap().messages.len(), 6);
    store
        .save_clean_delta(&id(), &transcript(8), 0, None)
        .await
        .unwrap();
    assert_eq!(store.load(&id()).await.unwrap().unwrap().messages.len(), 8);
}

#[path = "session_store_intact_tests.rs"]
mod intact_tests;
