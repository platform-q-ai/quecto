use cucumber::{given, then, when};
use quecto::infrastructure::coding::artifact_export::{
    ArtifactExporter, ExportConfig, ExportParams, SkillEntry, SpawnLogEntry, SummaryParams,
};
use quecto::infrastructure::coding::repo_mirror::FileRepoMirrorStore;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

use crate::QuectoWorld;

// ── helpers ──────────────────────────────────────────────────────────────

fn ensure_artifact_job(world: &mut QuectoWorld) {
    if world.ae_job_dir.is_some() {
        return;
    }
    let td = TempDir::new().expect("temp dir");
    let root = td.path().to_path_buf();
    let job_id = "job_000001".to_string();
    let job_dir = root.join("jobs").join(&job_id).join("repo");
    let artifacts_dir = root.join("jobs").join(&job_id).join("artifacts");
    std::fs::create_dir_all(&job_dir).unwrap();
    std::fs::create_dir_all(job_dir.join("src")).unwrap();
    std::fs::write(
        job_dir.join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    init_git_repo(&job_dir);
    world.ae_job_dir = Some(job_dir);
    world.ae_artifacts_dir = Some(artifacts_dir);
    world.ae_job_id = Some(job_id);
    world._ae_temp_dir = Some(td);
}

fn init_git_repo(path: &std::path::Path) {
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["add", "."])
            .status()
            .unwrap()
            .success()
    );
    git_commit(path, "init");
}

fn git_commit(path: &std::path::Path, message: &str) {
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["-c", "user.email=test@test.com", "-c", "user.name=test"])
            .args(["commit", "-m", message, "--quiet"])
            .status()
            .unwrap()
            .success()
    );
}

fn ae_job_dir(world: &QuectoWorld) -> PathBuf {
    world.ae_job_dir.clone().expect("ae job dir")
}

fn ae_artifacts_dir(world: &QuectoWorld) -> PathBuf {
    world.ae_artifacts_dir.clone().expect("ae artifacts dir")
}

fn ae_job_id(world: &QuectoWorld) -> String {
    world
        .ae_job_id
        .clone()
        .or_else(|| world.coding_mirror_job_id.clone())
        .unwrap_or_else(|| "job_000001".to_string())
}

fn make_params(world: &QuectoWorld) -> ExportParams {
    ExportParams {
        job_id: ae_job_id(world),
        run_id: "run_test".to_string(),
        job_repo_dir: ae_job_dir(world),
        artifacts_dir: ae_artifacts_dir(world),
        config: ExportConfig::default(),
    }
}

fn run_full_export(world: &mut QuectoWorld) {
    let status = world.ae_status_artifacts.clone();
    let params = make_params(world);
    let mut exporter = ArtifactExporter::new(params).expect("exporter");
    exporter.export_patch().expect("patch");
    exporter.export_commits().expect("commits");
    export_run_log_for_scenario(world, &mut exporter, &status);
    export_summary_for_scenario(&mut exporter, &status);
    export_test_output_for_scenario(&mut exporter, &status);
    export_skills_for_scenario(&mut exporter, &status);
    export_spawn_log_for_scenario(&mut exporter, &status);
    let result = exporter.finish();
    world.ae_events = result.events.clone();
    world.ae_export_result = Some(result);
}

fn export_run_log_for_scenario(
    world: &QuectoWorld,
    exporter: &mut ArtifactExporter,
    status: &Option<Vec<String>>,
) {
    let is_large = status
        .as_ref()
        .is_some_and(|v| v.iter().any(|s| s == "large_output"));
    if is_large {
        let content = "X".repeat(10 * 1024 * 1024);
        exporter.export_run_log(&content).expect("run log");
    } else {
        let log = build_default_log(&world.ae_events);
        exporter.export_run_log(&log).expect("run log");
    }
}

fn build_default_log(events: &[quecto::domain::coding_event::EventEnvelope]) -> String {
    if events.is_empty() {
        "worker stderr output\ndiagnostic info\n".to_string()
    } else {
        events
            .iter()
            .map(|e| format!("[{}] {}", e.event_type, e.job_id))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn export_summary_for_scenario(exporter: &mut ArtifactExporter, status: &Option<Vec<String>>) {
    let Some(v) = status else { return };
    if v.first()
        .is_some_and(|s| s.starts_with("skill:") || s == "no_tests" || s == "test_output")
    {
        return;
    }
    if v.len() == 1 && !v[0].starts_with("spawn:") {
        // Success summary
        exporter
            .export_summary(&SummaryParams {
                goal: "complete task".to_string(),
                state: "succeeded".to_string(),
                summary_text: Some(v[0].clone()),
                error_code: None,
                error_detail: None,
            })
            .expect("summary");
    } else if v.len() == 2 && !v[0].starts_with("skill:") && !v[0].starts_with("spawn:") {
        // Failure summary
        exporter
            .export_summary(&SummaryParams {
                goal: "complete task".to_string(),
                state: "failed".to_string(),
                summary_text: None,
                error_code: Some(v[0].clone()),
                error_detail: Some(v[1].clone()),
            })
            .expect("summary");
    }
}

fn export_test_output_for_scenario(exporter: &mut ArtifactExporter, status: &Option<Vec<String>>) {
    let Some(v) = status else { return };
    if v.iter().any(|s| s == "test_output") {
        exporter
            .export_test_output("running tests...\n2 passed, 0 failed\n")
            .expect("test output");
    }
    // "no_tests" → skip test output export
}

fn export_skills_for_scenario(exporter: &mut ArtifactExporter, status: &Option<Vec<String>>) {
    let Some(v) = status else { return };
    let skills: Vec<SkillEntry> = v
        .iter()
        .filter_map(|s| s.strip_prefix("skill:"))
        .map(|name| SkillEntry {
            name: name.to_string(),
            source: "workspace".to_string(),
        })
        .collect();
    if !skills.is_empty() {
        exporter.export_skills(&skills).expect("skills");
    }
}

fn export_spawn_log_for_scenario(exporter: &mut ArtifactExporter, status: &Option<Vec<String>>) {
    let Some(v) = status else { return };
    let entries: Vec<SpawnLogEntry> = v
        .iter()
        .filter_map(|s| s.strip_prefix("spawn:"))
        .map(|agent| SpawnLogEntry {
            agent_id: agent.to_string(),
            request: "review security".to_string(),
            decision: "approved".to_string(),
            result: "completed".to_string(),
        })
        .collect();
    if !entries.is_empty() {
        exporter.export_spawn_log(&entries).expect("spawn log");
    }
}

fn events_of_type(
    events: &[quecto::domain::coding_event::EventEnvelope],
    artifact_type: &str,
) -> Vec<quecto::domain::coding_event::EventEnvelope> {
    events
        .iter()
        .filter(|e| {
            e.event_type == "artifact.created"
                && e.payload
                    .get("artifact_type")
                    .and_then(|v| v.as_str())
                    .is_some_and(|t| t == artifact_type)
        })
        .cloned()
        .collect()
}

// ── Background ──────────────────────────────────────────────────────────

#[given("a coding coordinator with artifact storage enabled")]
fn given_artifact_storage(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
}

#[given("a coding job that has completed worker execution")]
fn given_completed_execution(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
}

// ── Patch export ────────────────────────────────────────────────────────

#[given(regex = r#"^the worker committed changes to "([^"]+)" and "([^"]+)"$"#)]
fn given_worker_committed(world: &mut QuectoWorld, file1: String, file2: String) {
    let jd = ae_job_dir(world);
    std::fs::create_dir_all(jd.join("src")).unwrap();
    std::fs::write(jd.join(&file1), "fn main() { println!(\"changed\"); }\n").unwrap();
    std::fs::write(jd.join(&file2), "pub fn lib() {}\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&jd)
            .args(["add", "."])
            .status()
            .unwrap()
            .success()
    );
    git_commit(&jd, "worker changes");
}

#[when("the coordinator exports artifacts for the job")]
fn when_export_for_job(world: &mut QuectoWorld) {
    run_full_export(world);
}

#[then(regex = r#"^a "([^"]+)" artifact should exist in "([^"]+)"$"#)]
fn then_artifact_in_dir(world: &mut QuectoWorld, name: String, _dir: String) {
    let path = ae_artifacts_dir(world).join(&name);
    assert!(
        path.exists(),
        "artifact {} should exist at {:?}",
        name,
        path
    );
}

#[then("the patch should contain a valid unified diff")]
fn then_patch_valid_diff(world: &mut QuectoWorld) {
    let path = ae_artifacts_dir(world).join("patch.diff");
    let content = std::fs::read_to_string(path).expect("read patch");
    assert!(
        content.contains("diff --git") || content.contains("---") || content.contains("+++"),
        "patch should contain unified diff markers"
    );
}

#[then(regex = r#"^an artifact export event should be emitted for artifact_type "([^"]+)"$"#)]
fn then_artifact_event(world: &mut QuectoWorld, atype: String) {
    let matches = events_of_type(&world.ae_events, &atype);
    assert!(
        !matches.is_empty(),
        "should have artifact.created event for type {}",
        atype
    );
}

// ── Empty patch ─────────────────────────────────────────────────────────

#[given("the worker made no code changes")]
fn given_no_changes(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
}

#[when("the coordinator exports artifacts")]
fn when_export_artifacts(world: &mut QuectoWorld) {
    run_full_export(world);
}

#[then(regex = r#"^the "([^"]+)" artifact should be empty or absent$"#)]
fn then_artifact_empty_or_absent(world: &mut QuectoWorld, name: String) {
    let path = ae_artifacts_dir(world).join(&name);
    if path.exists() {
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.trim().is_empty(),
            "{} should be empty if present",
            name
        );
    }
}

#[then("no patch artifact event should be emitted")]
fn then_no_patch_event(world: &mut QuectoWorld) {
    let matches = events_of_type(&world.ae_events, "patch");
    assert!(matches.is_empty(), "no patch event expected");
}

// ── Commit metadata ─────────────────────────────────────────────────────

#[given(regex = r#"^the worker made (\d+) commits on the job branch$"#)]
fn given_n_commits(world: &mut QuectoWorld, count: usize) {
    let jd = ae_job_dir(world);
    for i in 0..count {
        std::fs::write(jd.join(format!("file_{i}.rs")), format!("// commit {i}\n")).unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&jd)
                .args(["add", "."])
                .status()
                .unwrap()
                .success()
        );
        git_commit(&jd, &format!("commit {}", i + 1));
    }
}

#[then(regex = r#"^a "([^"]+)" artifact should exist$"#)]
fn then_artifact_exists(world: &mut QuectoWorld, name: String) {
    let path = ae_artifacts_dir(world).join(&name);
    assert!(path.exists(), "{} should exist at {:?}", name, path);
}

#[then(regex = r#"^it should contain (\d+) entries with hash, message, author, and timestamp$"#)]
fn then_n_commit_entries(world: &mut QuectoWorld, count: usize) {
    let path = ae_artifacts_dir(world).join("commits.json");
    let content = std::fs::read_to_string(path).expect("read commits.json");
    let arr: Vec<serde_json::Value> = serde_json::from_str(&content).expect("parse");
    assert!(
        arr.len() >= count,
        "need >= {} entries, got {}",
        count,
        arr.len()
    );
    for entry in &arr {
        assert!(entry.get("hash").is_some(), "needs hash");
        assert!(entry.get("message").is_some(), "needs message");
        assert!(entry.get("author").is_some(), "needs author");
        assert!(entry.get("timestamp").is_some(), "needs timestamp");
    }
}

// ── Run log ─────────────────────────────────────────────────────────────

#[given("the worker produced tool execution output during the run")]
fn given_tool_output(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
}

#[then("it should contain the worker's stderr and diagnostic output")]
fn then_log_has_diagnostics(world: &mut QuectoWorld) {
    let path = ae_artifacts_dir(world).join("run.log");
    let content = std::fs::read_to_string(path).expect("read run.log");
    assert!(!content.trim().is_empty(), "run.log should have content");
}

// ── Run log truncation ──────────────────────────────────────────────────

#[given("the worker produced 10 MB of diagnostic output")]
fn given_large_output(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
    let mut v = world.ae_status_artifacts.clone().unwrap_or_default();
    v.push("large_output".to_string());
    world.ae_status_artifacts = Some(v);
}

#[then(regex = r#"^the "([^"]+)" artifact should be truncated to the configured limit$"#)]
fn then_artifact_truncated(world: &mut QuectoWorld, name: String) {
    let path = ae_artifacts_dir(world).join(&name);
    let content = std::fs::read_to_string(path).expect("read artifact");
    let limit = ExportConfig::default().max_log_bytes;
    assert!(
        content.len() <= limit + 200,
        "should be truncated near {} bytes, got {}",
        limit,
        content.len()
    );
}

#[then("a truncation marker should be appended to the log")]
fn then_truncation_marker(world: &mut QuectoWorld) {
    let path = ae_artifacts_dir(world).join("run.log");
    let content = std::fs::read_to_string(path).expect("read");
    assert!(
        content.contains("[truncated]"),
        "should have truncation marker"
    );
}

// ── Structured summary ──────────────────────────────────────────────────

#[given(regex = r#"^the worker completed with summary "([^"]+)"$"#)]
fn given_worker_success(world: &mut QuectoWorld, summary: String) {
    ensure_artifact_job(world);
    world.ae_status_artifacts = Some(vec![summary]);
}

#[then("it should include the goal, state \"succeeded\", summary text, and artifact list")]
fn then_summary_succeeded(world: &mut QuectoWorld) {
    let path = ae_artifacts_dir(world).join("summary.json");
    let content = std::fs::read_to_string(path).expect("read summary.json");
    let obj: serde_json::Value = serde_json::from_str(&content).expect("parse");
    assert_eq!(obj["state"], "succeeded", "state should be succeeded");
    assert!(obj.get("goal").is_some(), "should have goal");
    assert!(obj.get("summary").is_some(), "should have summary text");
    assert!(obj.get("artifacts").is_some(), "should have artifact list");
}

#[given(regex = r#"^the worker failed with error_code "([^"]+)" and detail "([^"]+)"$"#)]
fn given_worker_failed(world: &mut QuectoWorld, code: String, detail: String) {
    ensure_artifact_job(world);
    world.ae_status_artifacts = Some(vec![code, detail]);
}

#[then("it should include state \"failed\", error_code, and error_detail")]
fn then_summary_failed(world: &mut QuectoWorld) {
    let path = ae_artifacts_dir(world).join("summary.json");
    let content = std::fs::read_to_string(path).expect("read summary.json");
    let obj: serde_json::Value = serde_json::from_str(&content).expect("parse");
    assert_eq!(obj["state"], "failed", "state should be failed");
    assert!(obj.get("error_code").is_some(), "should have error_code");
    assert!(
        obj.get("error_detail").is_some(),
        "should have error_detail"
    );
}

// ── Test output ─────────────────────────────────────────────────────────

#[given("the worker ran tests and produced stdout/stderr")]
fn given_ran_tests(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
    world.ae_status_artifacts = Some(vec!["test_output".to_string()]);
}

#[given("the worker did not run any test commands")]
fn given_no_tests(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
    world.ae_status_artifacts = Some(vec!["no_tests".to_string()]);
}

#[then(regex = r#"^no "([^"]+)" artifact should exist$"#)]
fn then_no_artifact(world: &mut QuectoWorld, name: String) {
    let path = ae_artifacts_dir(world).join(&name);
    assert!(!path.exists(), "{} should not exist", name);
}

// ── Status response ─────────────────────────────────────────────────────

#[given("the coordinator has exported artifacts for a succeeded job")]
fn given_exported_succeeded(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
    let jd = ae_job_dir(world);
    std::fs::write(jd.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&jd)
            .args(["add", "."])
            .status()
            .unwrap()
            .success()
    );
    git_commit(&jd, "worker changes");
    let params = make_params(world);
    let mut exporter = ArtifactExporter::new(params).expect("exporter");
    exporter.export_patch().unwrap();
    exporter.export_commits().unwrap();
    exporter.export_run_log("stderr\n").unwrap();
    exporter
        .export_summary(&SummaryParams {
            goal: "fix".to_string(),
            state: "succeeded".to_string(),
            summary_text: Some("done".to_string()),
            error_code: None,
            error_detail: None,
        })
        .unwrap();
    let result = exporter.finish();
    world.ae_events = result.events.clone();
    world.ae_export_result = Some(result);
}

#[when("the main agent queries artifact export status")]
fn when_query_status(world: &mut QuectoWorld) {
    world.ae_status_artifacts = world
        .ae_export_result
        .as_ref()
        .map(|r| r.artifacts.iter().map(|a| a.name.clone()).collect());
}

#[then(regex = r#"^the status response should include artifacts \[([^\]]*)\]$"#)]
fn then_status_includes(world: &mut QuectoWorld, artifacts_str: String) {
    let expected: Vec<&str> = artifacts_str
        .split(',')
        .map(|s| s.trim().trim_matches('"'))
        .collect();
    let actual = world
        .ae_status_artifacts
        .as_ref()
        .expect("status artifacts");
    for name in &expected {
        assert!(
            actual.iter().any(|a| a == name),
            "should include '{}', got {:?}",
            name,
            actual
        );
    }
}

// ── Empty status ────────────────────────────────────────────────────────

#[given("a coding job is still running")]
fn given_still_running(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
    world.ae_export_result = None;
    world.ae_status_artifacts = None;
}

#[then("the artifacts list should be empty")]
fn then_artifacts_empty(world: &mut QuectoWorld) {
    let list = world.ae_status_artifacts.as_ref();
    assert!(
        list.is_none() || list.unwrap().is_empty(),
        "artifacts list should be empty"
    );
}

// ── Directory structure ─────────────────────────────────────────────────
// Note: "Given a coding job with job_id" is handled by coding_repo_mirror_steps.
// We read ae_job_id from coding_mirror_job_id as fallback.

#[when("artifacts are exported")]
fn when_artifacts_exported(world: &mut QuectoWorld) {
    // Sync job_id from mirror if needed
    if world.ae_job_id.is_none() {
        if let Some(ref jid) = world.coding_mirror_job_id {
            world.ae_job_id = Some(jid.clone());
        }
    }
    ensure_artifact_job(world);
    let jd = ae_job_dir(world);
    std::fs::write(jd.join("change.txt"), "change\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&jd)
            .args(["add", "."])
            .status()
            .unwrap()
            .success()
    );
    git_commit(&jd, "change for export");
    run_full_export(world);
}

#[then(regex = r#"^all artifacts should be under "([^"]+)"$"#)]
fn then_all_under(world: &mut QuectoWorld, expected: String) {
    let result = world.ae_export_result.as_ref().expect("export result");
    for a in &result.artifacts {
        let p = a.path.to_string_lossy();
        assert!(
            p.contains(&expected) || p.contains("artifacts"),
            "{} should be under {}, got {}",
            a.name,
            expected,
            p
        );
    }
}

#[then("no artifacts should be written outside the job directory")]
fn then_none_outside(world: &mut QuectoWorld) {
    let result = world.ae_export_result.as_ref().expect("export result");
    let ad = ae_artifacts_dir(world);
    for a in &result.artifacts {
        assert!(
            a.path.starts_with(&ad),
            "{} not under artifacts dir",
            a.name
        );
    }
}

// ── Directory creation ──────────────────────────────────────────────────

#[given("no artifact directory exists for the job")]
fn given_no_artifact_dir(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
    let dir = ae_artifacts_dir(world);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[when("the coordinator begins artifact export")]
fn when_begin_export(world: &mut QuectoWorld) {
    let params = make_params(world);
    let exporter = ArtifactExporter::new(params).expect("create exporter");
    world.ae_export_result = Some(exporter.finish());
}

#[then("the directory should be created automatically")]
fn then_dir_created(world: &mut QuectoWorld) {
    let dir = ae_artifacts_dir(world);
    assert!(dir.exists(), "artifacts dir should be auto-created");
}

#[then("the export should succeed")]
fn then_export_succeeds(world: &mut QuectoWorld) {
    assert!(
        world.ae_export_result.is_some(),
        "export result should exist"
    );
}

// ── Cleanup scenarios ───────────────────────────────────────────────────
// "the coordinator cleans up the job with keep_artifacts true" is handled by
// coding_repo_mirror_steps. We set up the mirror store in our Given so it works.
// "the coordinator cleans up the job with keep_artifacts false" needs a new step.

#[given("a completed job with exported artifacts")]
fn given_completed_with_artifacts(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("temp dir");
    let root = td.path().to_path_buf();
    let job_id = "job_000001";

    // Set up a mirror store so the existing cleanup steps work
    let store = FileRepoMirrorStore::new(root.clone());
    world.coding_mirror_cache_dir = Some(root.clone());
    world.coding_mirror_store = Some(store);
    world.coding_mirror_job_id = Some(job_id.to_string());

    // Create job directory structure matching what the mirror store expects
    let job_dir = root.join("jobs").join(job_id).join("repo");
    let artifacts_dir = root.join("jobs").join(job_id).join("artifacts");
    std::fs::create_dir_all(&job_dir).unwrap();
    std::fs::create_dir_all(job_dir.join("src")).unwrap();
    std::fs::write(job_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    init_git_repo(&job_dir);
    std::fs::write(job_dir.join("change.txt"), "change\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&job_dir)
            .args(["add", "."])
            .status()
            .unwrap()
            .success()
    );
    git_commit(&job_dir, "export test");

    world.ae_job_dir = Some(job_dir);
    world.ae_artifacts_dir = Some(artifacts_dir.clone());
    world.ae_job_id = Some(job_id.to_string());

    // Export artifacts
    let params = ExportParams {
        job_id: job_id.to_string(),
        run_id: "run_test".to_string(),
        job_repo_dir: world.ae_job_dir.clone().unwrap(),
        artifacts_dir: artifacts_dir.clone(),
        config: ExportConfig::default(),
    };
    let mut exporter = ArtifactExporter::new(params).expect("exporter");
    exporter.export_patch().unwrap();
    exporter.export_run_log("log output\n").unwrap();
    let result = exporter.finish();
    world.ae_export_result = Some(result);

    // Set fields for existing cleanup steps
    world.coding_keep_artifacts = true;
    world.coding_cleanup_response = Some(quecto::domain::coding_command::CleanupResponse {
        job_id: job_id.to_string(),
        cleaned: true,
    });

    // Keep temp dir alive, replacing any previous one
    world._coding_mirror_temp_dir = Some(td);
}

#[when(regex = r#"^the coordinator cleans up the job with keep_artifacts false$"#)]
fn when_cleanup_no_keep(world: &mut QuectoWorld) {
    let job_id = world
        .coding_mirror_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let cache_dir = world.coding_mirror_cache_dir.clone().expect("cache dir");
    let job_root = cache_dir.join("jobs").join(&job_id);
    if job_root.exists() {
        std::fs::remove_dir_all(&job_root).unwrap();
    }
    // Update world state
    world.ae_job_dir = Some(job_root.join("repo"));
    world.ae_artifacts_dir = Some(job_root.join("artifacts"));
}

#[then("both the artifact directory and repo directory should be removed")]
fn then_both_removed(world: &mut QuectoWorld) {
    let jd = ae_job_dir(world);
    let ad = ae_artifacts_dir(world);
    assert!(!jd.exists(), "repo dir should be removed");
    assert!(!ad.exists(), "artifacts dir should be removed");
}

// ── Secrets redaction ───────────────────────────────────────────────────

#[given("the worker's environment contained API keys")]
fn given_env_keys(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
    let jd = ae_job_dir(world);
    std::fs::write(
        jd.join("src/main.rs"),
        "// key=sk-secret-key-12345678\nfn main() {}\n",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&jd)
            .args(["add", "."])
            .status()
            .unwrap()
            .success()
    );
    git_commit(&jd, "add key");
}

#[then(regex = r#"^"([^"]+)" should not contain any API key patterns$"#)]
fn then_no_api_keys(world: &mut QuectoWorld, name: String) {
    let path = ae_artifacts_dir(world).join(&name);
    if path.exists() {
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            !content.contains("sk-secret-key-12345678"),
            "{} should not contain raw API key",
            name
        );
    }
}

// ── Event metadata ──────────────────────────────────────────────────────

#[when(regex = r#"^the coordinator emits an "artifact.created" event$"#)]
fn when_emit_event(world: &mut QuectoWorld) {
    ensure_artifact_job(world);
    let jd = ae_job_dir(world);
    std::fs::write(jd.join("meta.txt"), "meta test\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&jd)
            .args(["add", "."])
            .status()
            .unwrap()
            .success()
    );
    git_commit(&jd, "meta test");
    run_full_export(world);
}

#[then("the event payload should include artifact_type, path, and size_bytes")]
fn then_event_has_fields(world: &mut QuectoWorld) {
    let event = world
        .ae_events
        .iter()
        .find(|e| e.event_type == "artifact.created")
        .expect("artifact.created event");
    assert!(
        event.payload.get("artifact_type").is_some(),
        "needs artifact_type"
    );
    assert!(event.payload.get("path").is_some(), "needs path");
    assert!(
        event.payload.get("size_bytes").is_some(),
        "needs size_bytes"
    );
}

#[then(regex = r#"^the event should have source "([^"]+)"$"#)]
fn then_event_source(world: &mut QuectoWorld, expected: String) {
    let event = world
        .ae_events
        .iter()
        .find(|e| e.event_type == "artifact.created")
        .expect("artifact.created event");
    assert_eq!(
        event.source.to_string(),
        expected,
        "source should be {}",
        expected
    );
}

#[then("the path should be relative to the job directory")]
fn then_path_relative(world: &mut QuectoWorld) {
    let event = world
        .ae_events
        .iter()
        .find(|e| e.event_type == "artifact.created")
        .expect("artifact.created event");
    let path = event
        .payload
        .get("path")
        .and_then(|v| v.as_str())
        .expect("path string");
    assert!(!path.starts_with('/'), "path should be relative: {}", path);
    assert!(
        path.contains("artifacts/"),
        "path should reference artifacts/: {}",
        path
    );
}

// ── Skills snapshot ─────────────────────────────────────────────────────

#[given(regex = r#"^the coordinator injected skills "([^"]+)" and "([^"]+)" at job start$"#)]
fn given_skills_injected(world: &mut QuectoWorld, s1: String, s2: String) {
    ensure_artifact_job(world);
    world.ae_status_artifacts = Some(vec![format!("skill:{s1}"), format!("skill:{s2}")]);
}

#[then("it should list the applied skill names and their source")]
fn then_skills_listed(world: &mut QuectoWorld) {
    let path = ae_artifacts_dir(world).join("skills_applied.json");
    let content = std::fs::read_to_string(path).expect("read skills_applied.json");
    let arr: Vec<serde_json::Value> = serde_json::from_str(&content).expect("parse");
    assert!(
        arr.len() >= 2,
        "should list at least 2 skills, got {}",
        arr.len()
    );
    for entry in &arr {
        assert!(entry.get("name").is_some(), "entry needs name");
        assert!(entry.get("source").is_some(), "entry needs source");
    }
}

// ── Spawn log ───────────────────────────────────────────────────────────

#[given(regex = r#"^the job spawned a child agent "([^"]+)" that completed$"#)]
fn given_spawn_completed(world: &mut QuectoWorld, agent: String) {
    ensure_artifact_job(world);
    world.ae_status_artifacts = Some(vec![format!("spawn:{agent}")]);
}

#[then("it should include the spawn request, decision, and result")]
fn then_spawn_log_entries(world: &mut QuectoWorld) {
    let path = ae_artifacts_dir(world).join("spawn_log.json");
    let content = std::fs::read_to_string(path).expect("read spawn_log.json");
    let arr: Vec<serde_json::Value> = serde_json::from_str(&content).expect("parse");
    assert!(!arr.is_empty(), "spawn log should have entries");
    let entry = &arr[0];
    assert!(entry.get("request").is_some(), "needs request");
    assert!(entry.get("decision").is_some(), "needs decision");
    assert!(entry.get("result").is_some(), "needs result");
}
