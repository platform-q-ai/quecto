use super::*;
use std::path::PathBuf;

fn args(s: &str) -> Vec<String> {
    let mut v = vec!["quecto-tui".to_string()];
    if !s.is_empty() {
        v.extend(s.split_whitespace().map(String::from));
    }
    v
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "quecto-tui-tabspawn-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn tab_spawn_registers_owned_watch_before_readiness_wait_completes() {
    let dir = tmp_dir("tab-pre-registration-race");
    let sock = dir.join("agent.sock");
    let script = dir.join("fake-agent.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '{} {}\\n'\nprintf 'quecto-agent-socket: {}\\n'\nsleep 30\n",
            quecto_line_io::PROTOCOL_ANNOUNCE_PREFIX,
            quecto_line_io::PROTOCOL_VERSION,
            sock.display()
        ),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&script, perms).unwrap();
    let _listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let flags = parse_flags(&args(""));
    let pending = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let program = script.to_str().unwrap().to_string();
    let spawn_flags = flags.clone();
    let spawn_pending = pending.clone();
    let spawn = tokio::spawn(async move {
        spawn_agent_program_watched_for_tab(&program, &spawn_flags, spawn_pending).await
    });

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let registered = loop {
        if let Some(watch) = pending.lock().unwrap().first().cloned() {
            break watch;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "TUI-owned tab spawn watch must be registered while readiness is still blocked"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };

    registered.terminate().await;
    let err = spawn
        .await
        .expect("spawn task must not panic")
        .expect_err("terminating the registered pre-readiness watch should abort startup");
    assert!(
        err.contains("agent exited before announcing socket")
            || err.contains("error reading agent stderr"),
        "unexpected startup error after terminating pre-readiness watch: {err}"
    );
}
