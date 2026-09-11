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

#[cfg(target_os = "linux")]
#[test]
fn registered_find_runtime_destruction_terminates_and_reaps() {
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    for multi_thread in [false, true] {
        for close_pipes in ["", "exec 1>&-", "exec 1>&- 2>&-"] {
            let dir = tempfile::tempdir().unwrap();
            let binary = dir.path().join("fd-fixture");
            let pidfile = dir.path().join("ready.pid");
            std::fs::write(
                &binary,
                format!(
                    "#!/bin/sh\n{close_pipes}\nprintf '%s' \"$$\" > '{}'\nwhile :; do :; done\n",
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
            let registry = build_official_tool_registry(
                tool,
                dir.path().to_path_buf(),
                sandbox,
                Default::default(),
            );
            let mut builder = if multi_thread {
                let mut builder = tokio::runtime::Builder::new_multi_thread();
                builder.worker_threads(2);
                builder
            } else {
                tokio::runtime::Builder::new_current_thread()
            };
            let runtime = builder.enable_all().build().unwrap();
            let mut invocation = Box::pin(registry.execute("find", r#"{"pattern":"*"}"#));
            let pid = runtime.block_on(async {
                tokio::select! {
                    result = &mut invocation => panic!("unexpected early result: {result:?}"),
                    pid = async {
                        let deadline = Instant::now() + Duration::from_secs(5);
                        loop {
                            if let Ok(text) = std::fs::read_to_string(&pidfile) {
                                if let Ok(pid) = text.parse::<i32>() { break pid; }
                            }
                            assert!(Instant::now() < deadline, "registered fixture readiness");
                            tokio::task::yield_now().await;
                        }
                    } => pid,
                }
            });
            // Cancel through actual registry dispatch immediately before teardown.
            drop(invocation);
            drop(runtime);
            let deadline = Instant::now() + Duration::from_secs(5);
            let process = std::path::PathBuf::from(format!("/proc/{pid}"));
            while process.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let disappeared = matches!(process.try_exists(), Ok(false));
            let mut status = 0;
            // SAFETY: targets only this readiness-identified child with a valid status pointer.
            let waited = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            let error = std::io::Error::last_os_error();
            if waited == 0 {
                // SAFETY: clean up only our still-owned child on regression failure.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                    libc::waitpid(pid, &mut status, 0);
                }
            }
            assert!(
                disappeared && waited == -1 && error.raw_os_error() == Some(libc::ECHILD),
                "registered cleanup: multi={multi_thread}, pipes={close_pipes:?}, pid={pid}, absent={disappeared}, waitpid={waited}, error={error}"
            );
        }
    }
}
