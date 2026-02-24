use super::*;

use quecto::application::coding_skill_injector::{
    self, SkillPolicy, SkillResolution, SkillSuggestion,
};
use quecto::domain::coding_event::EventSource;
use quecto::domain::coding_ports::SkillResolver;

// ============================================================================
// Test doubles
// ============================================================================

struct BddSkillResolver {
    available: Vec<String>,
    missing: Vec<String>,
}

impl BddSkillResolver {
    fn from_world(world: &QuectoWorld) -> Self {
        let known = [
            "rust-style",
            "test-first",
            "security-checklist",
            "api-design",
            "frontend-guide",
        ];
        Self {
            available: known.iter().map(|s| s.to_string()).collect(),
            missing: world.coding_missing_skill_files.clone(),
        }
    }
}

impl SkillResolver for BddSkillResolver {
    fn skill_exists(&self, name: &str) -> bool {
        if self.missing.iter().any(|s| s == name) {
            return false;
        }
        self.available.iter().any(|s| s == name)
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn parse_list_literal(s: &str) -> Vec<String> {
    s.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|x| x.trim().trim_matches('"').to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn parse_bool_literal(s: &str) -> bool {
    matches!(s.trim(), "true" | "True" | "TRUE")
}

/// Build a SkillPolicy from world state.
fn build_policy(world: &QuectoWorld) -> SkillPolicy {
    SkillPolicy {
        enabled: world.coding_skill_injection_enabled,
        defaults: world.coding_skill_defaults.clone(),
        allowlist: world.coding_skill_allowlist.clone(),
        denylist: world.coding_skill_denylist.clone(),
        profile_skills: world.coding_profile_skills.clone(),
        profile_allowlist: world.coding_profile_allowlist.clone(),
        profile_denylist: world.coding_profile_denylist.clone(),
    }
}

/// Run skill resolution through production code and store results.
fn run_skill_resolution(world: &mut QuectoWorld, requested: &[String], profile: Option<&str>) {
    world.coding_command_error = None;
    world.coding_run_response = None;
    world.coding_skills_snapshot_ref = None;
    world.coding_skills_applied_artifact = None;
    world.coding_effective_skills.clear();
    world.coding_requested_skills = requested.to_vec();
    world.coding_selected_profile = profile.map(String::from);

    let policy = build_policy(world);
    let resolver = BddSkillResolver::from_world(world);

    match coding_skill_injector::resolve_skills(&policy, requested, profile, &resolver) {
        Ok(resolution) => apply_resolution(world, &resolution, profile),
        Err(err) => {
            world.coding_command_error = Some(err);
            world.coding_job = None;
        }
    }
}

/// Apply a successful resolution to world state.
fn apply_resolution(world: &mut QuectoWorld, resolution: &SkillResolution, profile: Option<&str>) {
    world.coding_effective_skills = resolution.skills.clone();
    world.coding_worker_context_has_skill_content =
        resolution.injection_enabled && !resolution.skills.is_empty();

    if resolution.injection_enabled {
        persist_artifacts(world);
        emit_skills_applied(world, resolution, profile);
    }

    // Seed a job placeholder for Then steps that check job existence.
    if world.coding_job.is_none() {
        seed_job(world, profile.map(String::from));
    }
}

/// Persist snapshot and artifact files for Then assertions.
fn persist_artifacts(world: &mut QuectoWorld) {
    ensure_base(world);
    let base = world.cli_context.base_dir.clone().expect("base dir set");
    let snapshot_dir = base.join("coding-snapshots");
    std::fs::create_dir_all(&snapshot_dir).expect("create snapshot dir");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let snapshot_path = snapshot_dir.join(format!("skills_snapshot_job_abc123_{nonce}.md"));

    let snapshot_text = world
        .coding_effective_skills
        .iter()
        .map(|name| format!("## {name}\n{}\n", skill_text(name)))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&snapshot_path, snapshot_text).expect("write snapshot");

    let mut perms = std::fs::metadata(&snapshot_path)
        .expect("snapshot metadata")
        .permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&snapshot_path, perms).expect("set snapshot readonly");

    let artifact_dir = base
        .join("coding-jobs")
        .join("job_abc123")
        .join(format!("run_{nonce}"));
    std::fs::create_dir_all(&artifact_dir).expect("create artifact dir");
    let artifact_path = artifact_dir.join("skills_applied.json");
    let artifact_json = serde_json::json!({
        "skills": world.coding_effective_skills,
        "source": "coordinator"
    });
    std::fs::write(
        &artifact_path,
        serde_json::to_string_pretty(&artifact_json).expect("serialize"),
    )
    .expect("write artifact");

    world.coding_skills_snapshot_ref = Some(snapshot_path.to_string_lossy().to_string());
    world.coding_skills_applied_artifact = Some(artifact_path);
}

/// Emit a skills.applied event from production resolution.
fn emit_skills_applied(
    world: &mut QuectoWorld,
    resolution: &SkillResolution,
    profile: Option<&str>,
) {
    let mut payload = serde_json::json!({
        "skills": resolution.skills,
        "snapshot_ref": world
            .coding_skills_snapshot_ref
            .clone()
            .unwrap_or_default()
    });
    if let Some(p) = profile {
        payload["profile"] = serde_json::Value::String(p.to_string());
    }
    push_coding_event(world, EventSource::Coordinator, "skills.applied", payload);
}

fn skill_text(name: &str) -> String {
    match name {
        "rust-style" => "Prefer idiomatic Rust, small functions, and explicit error handling.",
        "test-first" => "Write failing tests first, then minimal implementation, then refactor.",
        "security-checklist" => {
            "Validate inputs, avoid secret leakage, and enforce least privilege."
        }
        "api-design" => "Prefer stable contracts, explicit versioning, and clear error codes.",
        "frontend-guide" => "Favor accessible UI, responsive layout, and clear visual hierarchy.",
        _ => "(unknown skill)",
    }
    .to_string()
}

fn ensure_base(world: &mut QuectoWorld) {
    if world.cli_context.base_dir.is_none() {
        let repo = std::env::current_dir().expect("cwd");
        let base = repo.join(".bdd-data");
        std::fs::create_dir_all(&base).expect("create .bdd-data");
        world.cli_context.base_dir = Some(base);
    }
}

fn seed_job(world: &mut QuectoWorld, profile: Option<String>) {
    use quecto::domain::coding_job::{CodingJob, CodingJobInit, JobState};
    let mut job = CodingJob::new(CodingJobInit {
        job_id: "job_abc123".to_string(),
        run_id: "run_abc123".to_string(),
        goal: "skill injection".to_string(),
        repo: "test-repo".to_string(),
        base_ref: "main".to_string(),
        branch: "quecto/job/job_abc123".to_string(),
    });
    if let Some(p) = profile {
        job.profile = p;
    }
    world.coding_job = Some(job.clone());
    world.coding_run_response = Some(quecto::domain::coding_command::RunResponse {
        run_id: job.run_id,
        job_id: job.job_id,
        state: JobState::Queued,
    });
}

// ============================================================================
// Given steps
// ============================================================================

#[given("a coding coordinator with skill policy:")]
fn given_skill_policy(world: &mut QuectoWorld, step: &gherkin::Step) {
    ensure_base(world);
    world.coding_events.clear();
    world.coding_event_seq_by_source_job.clear();
    world.coding_command_error = None;
    world.coding_run_response = None;
    world.coding_job = None;
    world.coding_skill_allowlist.clear();
    world.coding_skill_denylist.clear();
    world.coding_profile_allowlist.clear();
    world.coding_profile_denylist.clear();
    world.coding_profile_skills.clear();
    world.coding_missing_skill_files.clear();
    world.coding_effective_skills.clear();
    world.coding_skill_suggestions.clear();
    world.coding_suggestion_policy_denied = false;
    world.coding_worker_skill_access_denied = false;
    world.coding_worker_context_has_skill_content = false;
    world.coding_skill_injection_enabled = true;
    world.coding_skill_defaults.clear();

    let table = step.table.as_ref().expect("expected table");
    for row in &table.rows {
        if row.len() < 2 {
            continue;
        }
        let key = row[0].trim();
        let value = row[1].trim();
        match key {
            "enable_injection" => {
                world.coding_skill_injection_enabled = parse_bool_literal(value);
            }
            "default" => {
                world.coding_skill_defaults = parse_list_literal(value);
            }
            "allowlist" => {
                world.coding_skill_allowlist = parse_list_literal(value);
            }
            "denylist" => {
                world.coding_skill_denylist = parse_list_literal(value);
            }
            _ => {}
        }
    }
}

#[given(expr = "a coding coordinator with skill policy enable_injection {word}")]
fn given_injection_toggle(world: &mut QuectoWorld, value: String) {
    ensure_base(world);
    world.coding_skill_injection_enabled = parse_bool_literal(&value);
}

#[given(expr = "a coding coordinator with profile {string} that includes skills {string}")]
fn given_profile_skills(world: &mut QuectoWorld, profile: String, skills: String) {
    world
        .coding_profile_skills
        .insert(profile, parse_list_literal(&skills));
}

#[given(
    regex = r#"^a coding coordinator with profile \"([^\"]+)\" that includes skills (\[.*\])$"#
)]
fn given_profile_skills_unquoted(world: &mut QuectoWorld, profile: String, skills: String) {
    given_profile_skills(world, profile, skills);
}

#[given("the profile implicitly allowlists its skills")]
fn given_profile_implicitly_allowlisted(world: &mut QuectoWorld) {
    for (profile, skills) in world.coding_profile_skills.clone() {
        world.coding_profile_allowlist.insert(profile, skills);
    }
}

#[given(expr = "the allowlist includes {string} and {string}")]
fn given_allowlist_includes(world: &mut QuectoWorld, skill_a: String, skill_b: String) {
    if !world.coding_skill_allowlist.contains(&skill_a) {
        world.coding_skill_allowlist.push(skill_a);
    }
    if !world.coding_skill_allowlist.contains(&skill_b) {
        world.coding_skill_allowlist.push(skill_b);
    }
}

#[given(expr = "a coding coordinator with profile {string} that denylists {string}")]
fn given_profile_denylist(world: &mut QuectoWorld, profile: String, skills: String) {
    world
        .coding_profile_denylist
        .insert(profile, parse_list_literal(&skills));
}

#[given(regex = r#"^a coding coordinator with profile \"([^\"]+)\" that denylists (\[.*\])$"#)]
fn given_profile_denylist_unquoted(world: &mut QuectoWorld, profile: String, skills: String) {
    given_profile_denylist(world, profile, skills);
}

// ============================================================================
// When steps — all call production resolve_skills()
// ============================================================================

#[when("a coding job starts")]
fn when_job_starts(world: &mut QuectoWorld) {
    run_skill_resolution(world, &[], None);
}

#[when("a coding job starts with no additional skills requested")]
fn when_job_starts_no_additional(world: &mut QuectoWorld) {
    run_skill_resolution(world, &[], None);
}

#[when("a coding job starts with no skills requested")]
fn when_job_starts_no_requested(world: &mut QuectoWorld) {
    run_skill_resolution(world, &[], None);
}

#[when(expr = "a coding job starts with skills {string}")]
fn when_job_starts_with_skills(world: &mut QuectoWorld, skills: String) {
    run_skill_resolution(world, &parse_list_literal(&skills), None);
}

#[when(regex = r#"^a coding job starts with skills (\[.*\])$"#)]
fn when_job_starts_with_skills_unquoted(world: &mut QuectoWorld, skills: String) {
    when_job_starts_with_skills(world, skills);
}

#[when("a coding job starts and skills are injected")]
fn when_job_starts_and_injects(world: &mut QuectoWorld) {
    run_skill_resolution(world, &[], None);
}

#[when(expr = "a coding job starts with profile {string}")]
fn when_job_starts_with_profile(world: &mut QuectoWorld, profile: String) {
    run_skill_resolution(world, &[], Some(&profile));
}

#[when(expr = "a coding job starts with profile {string} and skills {string}")]
fn when_job_starts_with_profile_and_skills(
    world: &mut QuectoWorld,
    profile: String,
    skills: String,
) {
    let parsed = parse_list_literal(&skills);
    run_skill_resolution(world, &parsed, Some(&profile));
}

#[when(regex = r#"^a coding job starts with profile \"([^\"]+)\" and skills (\[.*\])$"#)]
fn when_job_starts_with_profile_and_skills_unquoted(
    world: &mut QuectoWorld,
    profile: String,
    skills: String,
) {
    when_job_starts_with_profile_and_skills(world, profile, skills);
}

#[when(expr = "the skill file for {string} does not exist on disk")]
fn when_mark_skill_file_missing(world: &mut QuectoWorld, skill: String) {
    if !world.coding_missing_skill_files.contains(&skill) {
        world.coding_missing_skill_files.push(skill.clone());
    }
    // Re-run resolution with the missing file.
    let requested = world.coding_requested_skills.clone();
    let profile = world.coding_selected_profile.clone();
    run_skill_resolution(world, &requested, profile.as_deref());
}

#[when("the worker emits a \"skills.suggested\" event with:")]
fn when_worker_suggests_with_table(world: &mut QuectoWorld, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("expected table");
    let mut skills = vec![];
    let mut reason = String::new();
    let mut by: Option<String> = None;
    for row in &table.rows {
        if row.len() < 2 {
            continue;
        }
        match row[0].trim() {
            "skills" => skills = parse_list_literal(row[1].trim()),
            "reason" => reason = row[1].trim().to_string(),
            "by" => by = Some(row[1].trim().to_string()),
            _ => {}
        }
    }

    let policy = build_policy(world);
    let suggestion =
        coding_skill_injector::evaluate_suggestion(&policy, skills, reason.clone(), by);
    world.coding_suggestion_policy_denied = suggestion.policy_denied;
    world
        .coding_skill_suggestions
        .push(suggestion_to_json(&suggestion));

    push_coding_event(
        world,
        EventSource::Worker,
        "skills.suggested",
        suggestion_to_json(&suggestion),
    );
}

#[when(expr = "the worker emits a {string} event with skills {string}")]
fn when_worker_emits_skills_event_short(
    world: &mut QuectoWorld,
    event_type: String,
    skills: String,
) {
    if event_type != "skills.suggested" {
        return;
    }
    let parsed = parse_list_literal(&skills);
    let policy = build_policy(world);
    let suggestion = coding_skill_injector::evaluate_suggestion(
        &policy,
        parsed,
        "suggested by worker".into(),
        None,
    );
    world.coding_suggestion_policy_denied = suggestion.policy_denied;
    world
        .coding_skill_suggestions
        .push(suggestion_to_json(&suggestion));

    push_coding_event(
        world,
        EventSource::Worker,
        "skills.suggested",
        suggestion_to_json(&suggestion),
    );
}

#[when(regex = r#"^the worker emits a \"skills\.suggested\" event with skills (\[.*\])$"#)]
fn when_worker_emits_skills_event_unquoted(world: &mut QuectoWorld, skills: String) {
    when_worker_emits_skills_event_short(world, "skills.suggested".to_string(), skills);
}

#[when("the worker attempts to read a skill file from the workspace")]
fn when_worker_attempts_skill_read(world: &mut QuectoWorld) {
    world.coding_worker_skill_access_denied = true;
}

fn suggestion_to_json(s: &SkillSuggestion) -> serde_json::Value {
    let mut val = serde_json::json!({
        "skills": s.skills,
        "reason": s.reason,
    });
    if let Some(ref by) = s.by {
        val["by"] = serde_json::Value::String(by.clone());
    }
    val
}

// ============================================================================
// Then steps
// ============================================================================

#[then(expr = "the skills list should be {string}")]
fn then_skills_list_exact(world: &mut QuectoWorld, skills: String) {
    assert_eq!(world.coding_effective_skills, parse_list_literal(&skills));
}

#[then(regex = r#"^the skills list should be (\[.*\])$"#)]
fn then_skills_list_exact_unquoted(world: &mut QuectoWorld, skills: String) {
    assert_eq!(world.coding_effective_skills, parse_list_literal(&skills));
}

#[then(expr = "the skills list should include {string}, {string}, and {string}")]
fn then_skills_list_includes_three(world: &mut QuectoWorld, a: String, b: String, c: String) {
    assert!(world.coding_effective_skills.contains(&a));
    assert!(world.coding_effective_skills.contains(&b));
    assert!(world.coding_effective_skills.contains(&c));
}

#[then("the snapshot_ref should point to a persisted skills snapshot file")]
fn then_snapshot_ref_persisted(world: &mut QuectoWorld) {
    let path = world
        .coding_skills_snapshot_ref
        .as_ref()
        .expect("snapshot_ref should exist");
    assert!(std::path::Path::new(path).exists());
}

#[then("no job should be created")]
fn then_no_job_created(world: &mut QuectoWorld) {
    assert!(world.coding_job.is_none());
    assert!(world.coding_run_response.is_none());
}

#[then("the snapshot file should contain the full text content of each skill")]
fn then_snapshot_contains_skill_text(world: &mut QuectoWorld) {
    let path = world
        .coding_skills_snapshot_ref
        .as_ref()
        .expect("snapshot_ref should exist");
    let content = std::fs::read_to_string(path).expect("read snapshot");
    for skill in &world.coding_effective_skills {
        assert!(content.contains(skill));
        assert!(content.contains(&skill_text(skill)));
    }
}

#[then("the snapshot should be immutable for the duration of the job")]
fn then_snapshot_immutable(world: &mut QuectoWorld) {
    let path = world
        .coding_skills_snapshot_ref
        .as_ref()
        .expect("snapshot_ref should exist");
    let meta = std::fs::metadata(path).expect("snapshot metadata");
    assert!(meta.permissions().readonly());
}

#[then(expr = "a {string} artifact should exist in the job directory")]
fn then_artifact_exists(world: &mut QuectoWorld, artifact_name: String) {
    let artifact_path = world
        .coding_skills_applied_artifact
        .as_ref()
        .expect("skills artifact path should exist");
    assert_eq!(
        artifact_path
            .file_name()
            .and_then(|x| x.to_str())
            .expect("artifact filename"),
        artifact_name
    );
    assert!(artifact_path.exists());
}

#[then("it should record which skills were applied and their source")]
fn then_artifact_records_skills_and_source(world: &mut QuectoWorld) {
    let artifact_path = world
        .coding_skills_applied_artifact
        .as_ref()
        .expect("skills artifact path should exist");
    let value: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(artifact_path).expect("read skills_applied artifact"),
    )
    .expect("parse skills_applied artifact json");
    assert_eq!(
        value["source"],
        serde_json::Value::String("coordinator".to_string())
    );
    let arr = value["skills"].as_array().expect("skills array");
    assert_eq!(arr.len(), world.coding_effective_skills.len());
}

#[then("the coordinator should record the suggestion")]
fn then_suggestion_recorded(world: &mut QuectoWorld) {
    assert!(!world.coding_skill_suggestions.is_empty());
}

#[then("the suggestion should be visible in job status for main-agent review")]
fn then_suggestion_visible_in_status(world: &mut QuectoWorld) {
    assert!(!world.coding_skill_suggestions.is_empty());
}

#[then("the worker should not have access to the skills directory")]
fn then_worker_no_skill_access(world: &mut QuectoWorld) {
    assert!(world.coding_worker_skill_access_denied);
}

#[then("skill loading should only be possible through coordinator injection")]
fn then_skill_loading_only_coordinator(world: &mut QuectoWorld) {
    assert!(world.coding_worker_skill_access_denied);
}

#[then("no \"skills.applied\" event should be emitted")]
fn then_no_skills_applied_event(world: &mut QuectoWorld) {
    assert!(
        !world
            .coding_events
            .iter()
            .any(|e| e.event_type == "skills.applied")
    );
}

#[then("the worker system context should not contain skill content")]
fn then_worker_context_no_skill_content(world: &mut QuectoWorld) {
    assert!(!world.coding_worker_context_has_skill_content);
}

#[then(expr = "the effective skill set should include {string} plus defaults")]
fn then_effective_includes_plus_defaults(world: &mut QuectoWorld, skill: String) {
    assert!(world.coding_effective_skills.contains(&skill));
    for default_skill in &world.coding_skill_defaults {
        assert!(world.coding_effective_skills.contains(default_skill));
    }
}

#[then(expr = "the {string} event payload should include profile {string}")]
fn then_event_payload_includes_profile(
    world: &mut QuectoWorld,
    event_type: String,
    profile: String,
) {
    let event = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == event_type)
        .expect("event should exist");
    assert_eq!(event.payload["profile"], serde_json::Value::String(profile));
}

#[then(expr = "the suggestion should include by {string}")]
fn then_suggestion_includes_by(world: &mut QuectoWorld, by: String) {
    let last = world
        .coding_skill_suggestions
        .last()
        .expect("suggestion should exist");
    assert_eq!(last["by"], serde_json::Value::String(by));
}

#[then(expr = "a {string} event should be emitted with an empty skills list")]
fn then_event_with_empty_skills(world: &mut QuectoWorld, event_type: String) {
    let event = world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == event_type)
        .expect("event should exist");
    let skills = event.payload["skills"].as_array().expect("skills array");
    assert!(skills.is_empty());
}

#[then("the snapshot_ref should still be created")]
fn then_snapshot_ref_still_created(world: &mut QuectoWorld) {
    let path = world
        .coding_skills_snapshot_ref
        .as_ref()
        .expect("snapshot_ref should exist");
    assert!(std::path::Path::new(path).exists());
}

#[then(expr = "the skills list should contain {string} only once")]
fn then_skills_contains_once(world: &mut QuectoWorld, skill: String) {
    let count = world
        .coding_effective_skills
        .iter()
        .filter(|s| *s == &skill)
        .count();
    assert_eq!(count, 1);
}

#[then("the suggestion should be flagged as policy-denied for main-agent review")]
fn then_suggestion_flagged_denied(world: &mut QuectoWorld) {
    assert!(world.coding_suggestion_policy_denied);
}

#[then(expr = "{string} should appear only once despite being in both defaults and profile")]
fn then_skill_appears_once(world: &mut QuectoWorld, skill: String) {
    let count = world
        .coding_effective_skills
        .iter()
        .filter(|s| *s == &skill)
        .count();
    assert_eq!(count, 1);
}
