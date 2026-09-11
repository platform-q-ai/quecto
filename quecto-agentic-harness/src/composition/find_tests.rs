use super::find::{build_find_tool, build_find_tool_with_binary};
use crate::infrastructure::extensions::native::build_official_tool_registry;
use crate::infrastructure::security::sandbox::Sandbox;
use std::sync::Arc;

#[tokio::test]
async fn convenience_registry_executes_find_and_preserves_default_session_contract() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("registered.txt"), "").unwrap();
    let sandbox = Sandbox::new(Some(dir.path().to_path_buf()));
    let tool = build_find_tool(
        Arc::new(dir.path().to_path_buf()),
        Arc::new(sandbox.clone()),
    );
    tool.set_session_key("find-composition".into());
    assert_eq!(tool.definition().name, "find");
    let registry =
        build_official_tool_registry(tool, dir.path().to_path_buf(), sandbox, Default::default());
    let result = registry
        .execute("find", r#"{"pattern":"registered.txt"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content, "registered.txt");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn registered_find_cancellation_terminates_and_reaps_child() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("fd-fixture");
    let pidfile = dir.path().join("ready.pid");
    // Shell is the direct child and busy loop creates no descendant process.
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nwhile :; do :; done\n",
            pidfile.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let sandbox = Sandbox::new(Some(dir.path().to_path_buf()));
    let tool = build_find_tool_with_binary(
        Arc::new(dir.path().to_path_buf()),
        Arc::new(sandbox.clone()),
        binary.to_string_lossy().into_owned(),
    );
    let registry = Arc::new(build_official_tool_registry(
        tool,
        dir.path().to_path_buf(),
        sandbox,
        Default::default(),
    ));
    let invocation =
        tokio::spawn(async move { registry.execute("find", r#"{"pattern":"*"}"#).await });
    let pid = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(text) = tokio::fs::read_to_string(&pidfile).await {
                if let Ok(pid) = text.parse::<u32>() {
                    break pid;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("fd readiness before cancellation");
    invocation.abort();
    assert!(invocation.await.unwrap_err().is_cancelled());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match tokio::fs::metadata(format!("/proc/{pid}")).await {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                _ => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("child disappears including zombie: termination AND reaping");
}
