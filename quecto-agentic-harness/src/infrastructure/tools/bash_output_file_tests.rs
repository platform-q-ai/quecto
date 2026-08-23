use super::*;
use tempfile::TempDir;

#[test]
fn test_output_file_summary_includes_timeout_qualifier() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("summary.txt");
    std::fs::write(&path, "one\ntwo\n").unwrap();
    let file = std::fs::OpenOptions::new().read(true).open(&path).unwrap();
    let target = OutputTarget { path, file };

    let summary = output_file_summary(&target, Some(" (partial)"));

    assert!(summary.contains(" (partial)"));
    assert!(summary.contains("bytes: 8"));
    assert!(summary.contains("lines: 2"));
}

#[test]
fn test_output_file_summary_falls_back_to_metadata_for_non_utf8() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("binary.bin");
    std::fs::write(&path, [0xff, 0xfe, b'a']).unwrap();
    let file = std::fs::OpenOptions::new().read(true).open(&path).unwrap();
    let target = OutputTarget { path, file };

    let summary = output_file_summary(&target, None);

    assert!(summary.contains("bytes: 3"));
    assert!(summary.contains("lines: 0"));
}

#[tokio::test]
async fn test_prepare_output_file_creates_parent_directories() {
    let tmp = TempDir::new().unwrap();
    let target = prepare_output_file(tmp.path(), "nested/out.txt")
        .await
        .unwrap();

    assert_eq!(target.path, tmp.path().join("nested/out.txt"));
    assert!(target.path.parent().unwrap().exists());
}

#[tokio::test]
async fn test_prepare_output_file_reports_directory_write_failure() {
    let tmp = TempDir::new().unwrap();

    let result = prepare_output_file(tmp.path(), ".").await;

    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("bash output_file failed")
    );
}

#[tokio::test]
async fn test_save_to_temp_file_uses_named_stable_location() {
    let path = save_to_temp_file("saved body".to_string()).await.unwrap();

    assert!(path.contains("quecto-bash-output"), "{}", path);
    assert!(path.contains("bash-output-"), "{}", path);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "saved body");
}

#[tokio::test]
async fn test_exec_output_file_with_env_overrides() {
    let (tool, tmp) = test_exec();
    let mut env = HashMap::new();
    env.insert("ISSUE_1518_VALUE".to_string(), "from-env".to_string());

    let result = tool
        .execute_with_env(
            r#"{"command": "printf \"$ISSUE_1518_VALUE\"", "output_file": "env.txt"}"#,
            &env,
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("env.txt")).unwrap(),
        "from-env"
    );
}

fn test_exec() -> (ExecTool, TempDir) {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()));
    let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));
    (tool, tmp)
}

fn test_exec_with_timeout(seconds: u64) -> (ExecTool, TempDir) {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()));
    let tool = ExecTool::with_timeout(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(sandbox),
        Duration::from_secs(seconds),
    );
    (tool, tmp)
}

#[tokio::test]
async fn test_exec_timeout_returns_captured_tail() {
    let (tool, _tmp) = test_exec_with_timeout(1);

    let result = tool
        .execute(r#"{"command": "python3 - <<'PY'\nimport sys, time\nfor i in range(6000):\n    print(f'line-{i}')\nprint('before-timeout')\nsys.stdout.flush()\ntime.sleep(60)\nPY"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("timed out"));
    assert!(
        result.content.contains("before-timeout"),
        "{}",
        result.content
    );
    assert!(
        result.content.len() <= 52 * 1024,
        "{}",
        result.content.len()
    );
    assert!(!result.content.contains("line-0\n"), "{}", result.content);
}

#[tokio::test]
async fn test_exec_timeout_with_output_file_writes_captured_output() {
    let (tool, tmp) = test_exec_with_timeout(1);

    let result = tool
        .execute(
            r#"{"command": "printf 'before-timeout\\n'; sleep 60", "output_file": "timeout.txt"}"#,
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("timed out"));
    assert!(result.content.contains("output saved to:"));
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("timeout.txt")).unwrap(),
        "before-timeout\n"
    );
}

#[tokio::test]
async fn test_exec_output_file_writes_full_combined_output_and_summarizes() {
    let (tool, tmp) = test_exec();
    let result = tool
        .execute(r#"{"command": "printf 'out\\n'; printf 'err\\n' >&2; exit 7", "output_file": "snapshots/out.txt"}"#)
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("exit code 7"));
    assert!(result.content.contains("output saved to:"));
    assert!(result.content.contains("bytes:"));
    assert!(result.content.contains("lines:"));
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("snapshots/out.txt")).unwrap(),
        "out\nerr\n"
    );
}

#[tokio::test]
async fn test_exec_output_file_keeps_large_output_out_of_inline_result() {
    let (tool, tmp) = test_exec();
    let result = tool
        .execute(r#"{"command": "python3 - <<'PY'\nprint('A' * 12000000)\nPY", "output_file": "large.txt"}"#)
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.len() < 4096, "{}", result.content.len());
    assert!(result.content.contains("bytes:"));
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("large.txt"))
            .unwrap()
            .trim(),
        "A".repeat(12000000)
    );
}
