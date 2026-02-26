use super::*;

fn args(flags: &[(&str, &str)]) -> Vec<String> {
    flags
        .iter()
        .flat_map(|(k, v)| vec![k.to_string(), v.to_string()])
        .collect()
}

#[test]
fn test_parse_all_required() {
    let a = args(&[
        ("--run-id", "r1"),
        ("--job-id", "j1"),
        ("--job-dir", "/tmp/x"),
        ("--goal", "fix"),
    ]);
    let result = parse_worker_args(&a).unwrap();
    assert_eq!(result.run_id, "r1");
    assert_eq!(result.job_id, "j1");
    assert_eq!(result.job_dir, "/tmp/x");
    assert_eq!(result.goal, "fix");
    assert!(result.model.is_none());
    assert!(result.max_iterations.is_none());
}

#[test]
fn test_parse_with_optional() {
    let a = args(&[
        ("--run-id", "r1"),
        ("--job-id", "j1"),
        ("--job-dir", "/tmp/x"),
        ("--goal", "fix"),
        ("--model", "gpt-4o"),
        ("--max-iterations", "50"),
    ]);
    let result = parse_worker_args(&a).unwrap();
    assert_eq!(result.model, Some("gpt-4o".to_string()));
    assert_eq!(result.max_iterations, Some(50));
}

#[test]
fn test_missing_run_id() {
    let a = args(&[
        ("--job-id", "j1"),
        ("--job-dir", "/tmp/x"),
        ("--goal", "fix"),
    ]);
    let err = parse_worker_args(&a).unwrap_err();
    assert!(err.contains("run-id"));
}

#[test]
fn test_missing_job_id() {
    let a = args(&[
        ("--run-id", "r1"),
        ("--job-dir", "/tmp/x"),
        ("--goal", "fix"),
    ]);
    let err = parse_worker_args(&a).unwrap_err();
    assert!(err.contains("job-id"));
}

#[test]
fn test_missing_job_dir() {
    let a = args(&[("--run-id", "r1"), ("--job-id", "j1"), ("--goal", "fix")]);
    let err = parse_worker_args(&a).unwrap_err();
    assert!(err.contains("job-dir"));
}

#[test]
fn test_missing_goal() {
    let a = args(&[
        ("--run-id", "r1"),
        ("--job-id", "j1"),
        ("--job-dir", "/tmp/x"),
    ]);
    let err = parse_worker_args(&a).unwrap_err();
    assert!(err.contains("goal"));
}

#[test]
fn test_unknown_flag() {
    let a = args(&[
        ("--run-id", "r1"),
        ("--job-id", "j1"),
        ("--job-dir", "/tmp/x"),
        ("--goal", "fix"),
        ("--bad-flag", "oops"),
    ]);
    let err = parse_worker_args(&a).unwrap_err();
    assert!(err.contains("bad-flag"));
}

#[test]
fn test_validate_existing_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    assert!(validate_job_dir(tmp.path().to_str().unwrap()).is_ok());
}

#[test]
fn test_validate_nonexistent_dir() {
    let err = validate_job_dir("/tmp/nonexistent-quecto-test-99999").unwrap_err();
    assert!(err.contains("does not exist"));
}

#[test]
fn test_cmd_worker_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let a = args(&[
        ("--run-id", "r1"),
        ("--job-id", "j1"),
        ("--job-dir", tmp.path().to_str().unwrap()),
        ("--goal", "fix"),
    ]);
    let mut stdout = String::new();
    let mut stderr = String::new();
    let code = cmd_worker(&a, &mut stdout, &mut stderr);
    assert_eq!(code, 0);
    assert!(stdout.contains("ready"));
}

#[test]
fn test_cmd_worker_bad_args() {
    let a: Vec<String> = vec![];
    let mut stdout = String::new();
    let mut stderr = String::new();
    let code = cmd_worker(&a, &mut stdout, &mut stderr);
    assert_eq!(code, 1);
    assert!(!stderr.is_empty());
}
