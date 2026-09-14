//! Current-behaviour contract of the flat session layout (#1970, R7 of
//! #1968): every path a session's identity projects to on disk comes from
//! one `FlatSessionLayout`, and the three file adapters (record store,
//! ownership stamp, retention file) write exactly where the raw-key code
//! wrote before — `<base>/sessions/cli_alpha.json`, `.owner` and
//! `cli_alpha/spill.jsonl` for `cli:alpha` — through the same sanitizer,
//! with the ephemeral identity keeping its sanitized-empty-key spill file
//! and distinct identities never sharing a projection.
//!
//! This pins the seam a folder/workspace-scoped store changes later: the
//! identity and this one projection, not the callers.

use quecto::application::sessions::ports::{ContextSpillStore, SessionStore};
use quecto::domain::message::Message;
use quecto::domain::session::{Session, SpillEntry};
use quecto::domain::session_identity::{SessionIdentity, SpillId};
use quecto::infrastructure::persistence::context_spill::FileContextSpillStore;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;

fn spill(id: &str) -> SpillEntry {
    SpillEntry {
        id: id.to_string(),
        tool: "bash".to_string(),
        input_preview: "ls".to_string(),
        tokens: 1,
        content: format!("content {id}"),
    }
}

#[tokio::test]
async fn cli_alpha_lands_at_the_exact_unchanged_paths_through_the_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path();
    let layout = FlatSessionLayout::new(base);
    let alpha = SessionIdentity::named_cli("alpha").unwrap();

    let store = FileSessionStore::new(layout.clone());
    let mut session = Session::new(alpha.clone());
    session.messages.push(Message::user("hello"));
    store.claim(&alpha).unwrap();
    store.save(&session).await.unwrap();
    let retention = FileContextSpillStore::new(layout.clone());
    retention
        .append(&alpha, &spill("turn1:bash:0"))
        .await
        .unwrap();

    let record = base.join("sessions/cli_alpha.json");
    let stamp = base.join("sessions/cli_alpha.owner");
    let spill_file = base.join("sessions/cli_alpha/spill.jsonl");
    assert!(record.is_file(), "{}", record.display());
    assert!(stamp.is_file(), "{}", stamp.display());
    assert!(spill_file.is_file(), "{}", spill_file.display());
    assert_eq!(layout.session_file(&alpha), record);
    assert_eq!(layout.ownership_stamp(&alpha), stamp);
    assert_eq!(layout.spill_file(&alpha), spill_file);

    let entries: Vec<_> = std::fs::read_dir(base.join("sessions"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    let mut entries = entries;
    entries.sort();
    assert_eq!(
        entries,
        ["cli_alpha", "cli_alpha.json", "cli_alpha.owner"],
        "nothing but the three projections is written"
    );
}

#[tokio::test]
async fn unsafe_and_unicode_keys_round_trip_through_the_same_sanitizer() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(tmp.path());
    let store = FileSessionStore::new(layout.clone());
    let retention = FileContextSpillStore::new(layout.clone());
    for key in ["../escape", "chät", "with space", "telegram:12345"] {
        let identity = SessionIdentity::from_persisted_key(key);
        let mut session = Session::new(identity.clone());
        session.messages.push(Message::user(key));
        store.save(&session).await.unwrap();
        retention.append(&identity, &spill("x")).await.unwrap();

        let record = layout.session_file(&identity);
        assert!(record.starts_with(tmp.path().join("sessions")));
        assert_eq!(record.parent().unwrap(), tmp.path().join("sessions"));
        assert!(record.is_file(), "{key}: {}", record.display());
        assert!(layout.spill_file(&identity).is_file(), "{key}");
        let loaded = store.load(&identity).await.unwrap().unwrap();
        assert_eq!(loaded.key, identity, "{key} round-trips");
        assert_eq!(
            retention
                .recall(&identity, &SpillId::new("x"))
                .await
                .unwrap()
                .map(|e| e.content),
            Some("content x".to_string())
        );
    }
}

#[tokio::test]
async fn ephemeral_spill_keeps_the_sanitized_empty_key_file_and_scrubs_it() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(tmp.path());
    let retention = FileContextSpillStore::new(layout.clone());
    let ephemeral = SessionIdentity::ephemeral();
    retention.append(&ephemeral, &spill("run")).await.unwrap();
    let file = tmp.path().join("sessions/key_/spill.jsonl");
    assert!(file.is_file());
    assert_eq!(layout.spill_file(&ephemeral), file);

    FileContextSpillStore::scrub_session_spill_sync(&layout, &ephemeral);
    assert!(!file.exists(), "scrub removes the ephemeral retention file");
    assert!(
        !file.parent().unwrap().exists(),
        "and its now-empty directory"
    );
}

#[tokio::test]
async fn distinct_identities_are_isolated_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(tmp.path());
    let store = FileSessionStore::new(layout.clone());
    let retention = FileContextSpillStore::new(layout.clone());
    let a = SessionIdentity::named_cli("a").unwrap();
    let b = SessionIdentity::named_cli("b").unwrap();
    let mut session_a = Session::new(a.clone());
    session_a.messages.push(Message::user("only a"));
    store.save(&session_a).await.unwrap();
    retention.append(&a, &spill("only-a")).await.unwrap();

    assert!(store.load(&b).await.unwrap().is_none());
    assert!(!store.exists(&b).await.unwrap());
    assert!(retention.list_entries(&b).await.unwrap().is_empty());
    assert!(
        retention
            .recall(&b, &SpillId::new("only-a"))
            .await
            .unwrap()
            .is_none()
    );
    assert_ne!(layout.session_file(&a), layout.session_file(&b));
    assert_ne!(layout.spill_file(&a), layout.spill_file(&b));
}
