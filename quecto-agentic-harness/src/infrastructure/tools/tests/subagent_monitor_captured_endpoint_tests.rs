use crate::infrastructure::tools::{
    subagent_monitor::{MonitorContext, spawn_monitor_task},
    subagent_registry,
};

fn test_entry() -> subagent_registry::SubagentEntry {
    subagent_registry::SubagentEntry::new(std::path::PathBuf::from("/tmp/test.sock"), 123)
}

#[tokio::test]
async fn monitor_uses_captured_proxy_endpoint_even_if_registry_entry_races() {
    use tokio::io::AsyncWriteExt;
    let dir = tempfile::tempdir().unwrap();
    let requested = dir.path().join("requested-direct.sock");
    let proxy = dir.path().join("proxy-only.sock");
    let listener = tokio::net::UnixListener::bind(&proxy).unwrap();
    let registry = subagent_registry::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child".to_string(), test_entry());
    let handle = spawn_monitor_task(
        "child".to_string(),
        crate::domain::agent_launch_backend::ParentEndpoint::Proxy(format!(
            "unix://{}",
            proxy.display()
        )),
        registry.clone(),
        None,
        MonitorContext::default(),
    );
    registry.lock().unwrap().remove("child");
    let (mut stream, _) =
        tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
            .await
            .expect("monitor must connect to captured proxy endpoint, not registry/requested UDS")
            .unwrap();
    stream
        .write_all(b"{\"type\":\"agent_end\"}\n")
        .await
        .unwrap();
    assert!(
        tokio::net::UnixStream::connect(&requested).await.is_err(),
        "test must remain proxy-only with no requested direct UDS listener"
    );
    handle.abort();
}

#[tokio::test]
async fn monitor_uses_captured_proxy_endpoint_when_registry_entry_removed_before_start() {
    use crate::domain::agent_launch_backend::ParentEndpoint;
    use tokio::io::AsyncWriteExt;

    let dir = tempfile::tempdir().unwrap();
    let requested = dir.path().join("requested-never-created.sock");
    let proxy = dir.path().join("proxy.sock");
    let listener = tokio::net::UnixListener::bind(&proxy).unwrap();
    let registry = subagent_registry::new_registry();
    let mut entry = test_entry();
    entry.socket_path = requested.clone();
    entry.parent_endpoint = Some(ParentEndpoint::Proxy(format!("unix:{}", proxy.display())));
    entry.socket_mode = Some("proxy".to_string());
    registry.lock().unwrap().insert("child".to_string(), entry);

    let handle = spawn_monitor_task(
        "child".to_string(),
        ParentEndpoint::Proxy(format!("unix:{}", proxy.display())),
        registry.clone(),
        None,
        MonitorContext::default(),
    );
    registry.lock().unwrap().remove("child");

    let (mut stream, _) =
        tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
            .await
            .expect("monitor must connect through captured proxy, not registry/direct fallback")
            .unwrap();
    stream
        .write_all(b"{\"type\":\"agent_start\"}\n")
        .await
        .unwrap();
    drop(stream);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), handle).await;
    assert!(
        !requested.exists(),
        "requested direct UDS remains absent in proxy-only mode"
    );
}
