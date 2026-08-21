use super::*;
use std::fs;

fn ws(id: &str, tabs: Vec<WorkspaceTabEntry>, active: usize) -> WorkspaceManifest {
    WorkspaceManifest {
        workspace_id: id.into(),
        label: String::new(),
        last_active_unix_s: 0,
        active_index: active,
        tabs,
        updated_unix_s: 42,
    }
}

#[test]
fn store_load_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifests.json");
    let mut store = WorkspaceManifestStore::new();
    store.upsert(ws(
        "ws-a",
        vec![
            WorkspaceTabEntry {
                tab_id: 0,
                session_key: Some("s0".into()),
                name: Some("main".into()),
                summary: None,
            },
            WorkspaceTabEntry {
                tab_id: 1,
                session_key: Some("s1".into()),
                name: None,
                summary: None,
            },
        ],
        1,
    ));
    store.store(&path).unwrap();
    let loaded = WorkspaceManifestStore::load(&path);
    assert_eq!(loaded.workspaces.len(), 1);
    let m = loaded.get("ws-a").unwrap();
    assert_eq!(m.active_index, 1);
    assert_eq!(m.tabs.len(), 2);
    assert_eq!(m.tabs[0].session_key.as_deref(), Some("s0"));
}

#[test]
fn missing_and_corrupt_load_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        WorkspaceManifestStore::load(&dir.path().join("x.json"))
            .workspaces
            .is_empty()
    );
    let path = dir.path().join("m.json");
    fs::write(&path, b"[").unwrap();
    assert!(WorkspaceManifestStore::load(&path).workspaces.is_empty());
    fs::write(&path, br#"{"version":1,"workspaces":"#).unwrap();
    assert!(WorkspaceManifestStore::load(&path).workspaces.is_empty());
}

#[test]
fn active_index_clamped_when_out_of_range_on_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.json");
    fs::write(
        &path,
        br#"{"version":1,"workspaces":[{"workspace_id":"w","active_index":99,"tabs":[{"tab_id":0,"session_key":null,"name":null}],"updated_unix_s":1}]}"#,
    )
    .unwrap();
    let loaded = WorkspaceManifestStore::load(&path);
    assert_eq!(loaded.workspaces[0].active_index, 0);
}

#[test]
fn load_accepts_legacy_session_key_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.json");
    fs::write(
        &path,
        br#"{"version":1,"workspaces":[{"workspace_id":"w","label":"","last_active_unix_s":0,"active_index":0,"tabs":[{"tab_id":0,"sessionKey":"legacy-session","name":"main"}],"updated_unix_s":1}]}"#,
    )
    .unwrap();

    let loaded = WorkspaceManifestStore::load(&path);
    assert_eq!(
        loaded.workspaces[0].tabs[0].session_key.as_deref(),
        Some("legacy-session"),
        "legacy manifests using sessionKey must remain resumable"
    );

    fs::write(
        &path,
        br#"{"version":1,"workspaces":[{"workspace_id":"w","label":"","last_active_unix_s":0,"active_index":0,"tabs":[{"tab_id":0,"session":"older-session","name":"main"}],"updated_unix_s":1}]}"#,
    )
    .unwrap();
    let loaded = WorkspaceManifestStore::load(&path);
    assert_eq!(
        loaded.workspaces[0].tabs[0].session_key.as_deref(),
        Some("older-session"),
        "older manifests using session must remain resumable"
    );
}

#[test]
fn upsert_and_remove() {
    let mut store = WorkspaceManifestStore::new();
    store.upsert(ws("a", vec![], 0));
    store.upsert(ws("b", vec![], 0));
    assert!(store.remove("a"));
    assert!(!store.remove("a"));
    assert!(store.get("b").is_some());
    assert!(store.get("a").is_none());
}

#[test]
fn atomic_replace_leaves_readable_complete_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.json");
    let mut store = WorkspaceManifestStore::new();
    store.upsert(ws("w", vec![], 0));
    store.store(&path).unwrap();
    store.upsert(ws(
        "w",
        vec![WorkspaceTabEntry {
            tab_id: 0,
            session_key: Some("k".into()),
            name: Some("n".into()),
            summary: None,
        }],
        0,
    ));
    store.store(&path).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["workspaces"][0]["tabs"][0]["session_key"], "k");
}

#[test]
fn default_manifest_path_uses_data_dir() {
    let p = default_manifest_path();
    assert!(p.ends_with(DEFAULT_MANIFEST_FILE_NAME));
    assert!(p.to_string_lossy().contains("quecto"));
}
