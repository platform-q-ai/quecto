use crate::infrastructure::tools::spawn::SpawnTool;

#[tokio::test]
#[serial_test::serial]
async fn launch_uds_agent_rolls_back_after_initial_prompt_send_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let child = dir.path().join("prompt-fail-child.py");
    let pid_file = dir.path().join("child.pid");
    std::fs::write(
        &child,
        format!(
            r#"#!/usr/bin/env python3
import os, signal, socket, sys, time
with open({pid_file:?}, "w") as f:
    f.write(str(os.getpid()))
    f.flush()
sock_path = sys.argv[sys.argv.index("--socket") + 1]
try:
    os.unlink(sock_path)
except FileNotFoundError:
    pass
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sock_path)
s.listen(1)
conn, _ = s.accept()
conn.close()
time.sleep(30)
"#,
            pid_file = pid_file.to_string_lossy().to_string()
        ),
    )
    .expect("write fake child");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&child).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&child, perms).unwrap();
    }

    let old_child = std::env::var_os("QUECTO_CHILD_BINARY");
    // SAFETY: serialized test restores this process-wide override before return.
    unsafe { std::env::set_var("QUECTO_CHILD_BINARY", &child) };
    let tool = SpawnTool::with_base_dir(vec![], true, dir.path().to_path_buf())
        .with_socket_dir(dir.path().to_path_buf());
    let cfg = tool
        .parse_args(r#"{"agent_id":"prompt-fail","task":"must fail after registration"}"#)
        .unwrap();

    let err = tool
        .launch_uds_agent(&cfg)
        .await
        .expect_err("closed prompt socket must fail launch transaction");
    match old_child {
        Some(value) => {
            // SAFETY: restores the serialized test-scoped environment override.
            unsafe { std::env::set_var("QUECTO_CHILD_BINARY", value) }
        }
        None => {
            // SAFETY: restores the serialized test-scoped environment override.
            unsafe { std::env::remove_var("QUECTO_CHILD_BINARY") }
        }
    }

    assert!(
        err.to_string()
            .contains("failed to send prompt to subagent"),
        "caller must receive original actionable prompt-send error: {err}"
    );
    assert!(
        tool.registry().lock().unwrap().is_empty(),
        "registered subagent entry must be rolled back"
    );

    if let Ok(pid_text) = std::fs::read_to_string(&pid_file) {
        let pid: u32 = pid_text.parse().expect("pid");
        #[cfg(unix)]
        {
            for _ in 0..50 {
                // SAFETY: kill(pid, 0) probes process existence without sending a signal.
                let alive = unsafe { libc::kill(pid as i32, 0) == 0 };
                if !alive {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            panic!("child process {pid} remained live after rollback");
        }
    }
}
