use super::*;
use std::fs;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

fn sample(tab_id: u32, socket: PathBuf, status: TabAgentStatus) -> TabAgentRecord {
    TabAgentRecord {
        tab_id,
        pid: Some(std::process::id()),
        socket_path: socket,
        session_key: Some(format!("sess-{tab_id}")),
        tab_name: Some(format!("tab-{tab_id}")),
        workspace_id: Some("ws-1".into()),
        updated_unix_s: 1,
        status,
    }
}

#[test]
fn store_then_load_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.json");
    let mut reg = TabAgentRegistry::new();
    reg.upsert(sample(
        0,
        PathBuf::from("/tmp/a.sock"),
        TabAgentStatus::Live,
    ));
    reg.store(&path).unwrap();
    let loaded = TabAgentRegistry::load(&path);
    assert_eq!(loaded.agents.len(), 1);
    assert_eq!(loaded.agents[0].tab_id, 0);
    assert_eq!(loaded.agents[0].session_key.as_deref(), Some("sess-0"));
}

#[test]
fn missing_file_loads_empty() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = TabAgentRegistry::load(&dir.path().join("nope.json"));
    assert!(loaded.agents.is_empty());
    assert_eq!(loaded.version, REGISTRY_SCHEMA_VERSION);
}

#[test]
fn corrupt_and_partial_json_loads_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.json");
    fs::write(&path, b"{not-json").unwrap();
    assert!(TabAgentRegistry::load(&path).agents.is_empty());
    fs::write(&path, br#"{"version":1,"agents":[{"tab_id":0"#).unwrap();
    assert!(TabAgentRegistry::load(&path).agents.is_empty());
    fs::write(&path, br#"{"version":999,"agents":[]}"#).unwrap();
    assert!(TabAgentRegistry::load(&path).agents.is_empty());
}

#[test]
fn gc_dead_removes_dead_and_probe_false_retains_live() {
    let mut reg = TabAgentRegistry::new();
    reg.upsert(sample(
        0,
        PathBuf::from("/tmp/live.sock"),
        TabAgentStatus::Live,
    ));
    reg.upsert(sample(
        1,
        PathBuf::from("/tmp/dead.sock"),
        TabAgentStatus::Dead,
    ));
    reg.upsert(sample(
        2,
        PathBuf::from("/tmp/probe-dead.sock"),
        TabAgentStatus::Live,
    ));
    reg.gc_dead(|r| r.tab_id == 0);
    assert_eq!(reg.agents.len(), 1);
    assert_eq!(reg.agents[0].tab_id, 0);
}

#[test]
fn gc_with_default_probe_retains_live_socket_and_drops_missing() {
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("live.sock");
    let _listener = UnixListener::bind(&sock_path).unwrap();
    let mut reg = TabAgentRegistry::new();
    reg.upsert(sample(0, sock_path.clone(), TabAgentStatus::Live));
    reg.upsert(sample(
        1,
        dir.path().join("missing.sock"),
        TabAgentStatus::Live,
    ));
    reg.gc_dead(default_liveness_probe);
    assert_eq!(reg.agents.len(), 1);
    assert_eq!(reg.agents[0].tab_id, 0);
    assert_eq!(reg.agents[0].socket_path, sock_path);
}

#[test]
fn refresh_status_marks_dead_without_removing() {
    let mut reg = TabAgentRegistry::new();
    reg.upsert(sample(0, PathBuf::from("/tmp/a"), TabAgentStatus::Unknown));
    reg.upsert(sample(1, PathBuf::from("/tmp/b"), TabAgentStatus::Unknown));
    reg.refresh_status(|r| r.tab_id == 0);
    assert_eq!(reg.agents[0].status, TabAgentStatus::Live);
    assert_eq!(reg.agents[1].status, TabAgentStatus::Dead);
    assert_eq!(reg.agents.len(), 2);
}

#[test]
fn upsert_replaces_same_tab_stable_identity() {
    let mut reg = TabAgentRegistry::new();
    reg.upsert(sample(0, PathBuf::from("/tmp/a"), TabAgentStatus::Live));
    let mut next = sample(0, PathBuf::from("/tmp/b"), TabAgentStatus::Live);
    next.tab_name = Some("renamed".into());
    reg.upsert(next);
    assert_eq!(reg.agents.len(), 1);
    assert_eq!(reg.agents[0].tab_name.as_deref(), Some("renamed"));
    assert_eq!(reg.agents[0].socket_path, PathBuf::from("/tmp/b"));
}

#[test]
fn upsert_preserves_detached_master_with_different_session_key() {
    let mut reg = TabAgentRegistry::new();
    let mut detached = sample(
        0,
        PathBuf::from("/tmp/old-master.sock"),
        TabAgentStatus::Live,
    );
    detached.session_key = Some("cli:work".into());
    reg.upsert(detached);

    let mut fresh = sample(
        0,
        PathBuf::from("/tmp/new-master.sock"),
        TabAgentStatus::Live,
    );
    fresh.session_key = Some("cli:new-master".into());
    reg.upsert(fresh);

    assert_eq!(
        reg.agents.len(),
        2,
        "same tab id across TUI lifetimes must not erase the detached live owner"
    );
    assert!(reg.agents.iter().any(|a| a.tab_id == 0
        && a.session_key.as_deref() == Some("cli:work")
        && a.socket_path == std::path::Path::new("/tmp/old-master.sock")));
    assert!(reg.agents.iter().any(|a| a.tab_id == 0
        && a.session_key.as_deref() == Some("cli:new-master")
        && a.socket_path == std::path::Path::new("/tmp/new-master.sock")));
}

#[test]
fn default_registry_path_uses_xdg_or_home() {
    let p = default_registry_path();
    assert!(p.ends_with(DEFAULT_REGISTRY_FILE_NAME));
    assert!(p.to_string_lossy().contains("quecto"));
}
