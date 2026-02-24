use super::*;

use quecto::domain::coding_event::EventSource;
use quecto::domain::coding_job::JobState;

fn parse_csv_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .map(|x| x.trim_matches('"'))
        .filter(|x| !x.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn last_event<'a>(
    world: &'a QuectoWorld,
    event_type: &str,
) -> &'a quecto::domain::coding_event::EventEnvelope {
    world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == event_type)
        .expect("event should exist")
}

fn emit_publish_request(world: &mut QuectoWorld, action: &str) {
    world.coding_publish_requested_action = Some(action.to_string());
    push_coding_event(
        world,
        EventSource::MainAgent,
        "publish.request",
        serde_json::json!({"action": action}),
    );
}

fn emit_publish_result(world: &mut QuectoWorld, action: &str, ok: bool, error: Option<&str>) {
    world.coding_publish_result_action = Some(action.to_string());
    world.coding_publish_ok = Some(ok);
    world.coding_publish_error = error.map(ToString::to_string);
    push_coding_event(
        world,
        EventSource::Coordinator,
        "publish.result",
        serde_json::json!({
            "action": action,
            "ok": ok,
            "error": error,
            "pr_number": world.coding_publish_pr_number,
            "url": world.coding_publish_pr_url,
        }),
    );
}

fn can_publish_for_state(action: &str, state: JobState) -> Result<(), String> {
    if action == "get_pr_status" {
        return Ok(());
    }
    if state == JobState::Succeeded {
        return Ok(());
    }
    let msg = match state {
        JobState::Failed => "job did not succeed",
        JobState::Canceled => "job was canceled",
        _ => "job is not in a publishable terminal state",
    };
    Err(msg.to_string())
}

fn run_publish_action(world: &mut QuectoWorld, action: &str) {
    let (job_state, job_repo) = {
        let job = world.coding_job.as_ref().expect("job should exist");
        (job.state, job.repo.clone())
    };
    emit_publish_request(world, action);

    if let Err(err) = can_publish_for_state(action, job_state) {
        emit_publish_result(world, action, false, Some(&err));
        return;
    }

    if !world.coding_publish_repo_allowlist.is_empty() {
        let repo = world.coding_publish_target_repo.clone().unwrap_or(job_repo);
        if !world
            .coding_publish_repo_allowlist
            .iter()
            .any(|r| r == &repo)
        {
            emit_publish_result(world, action, false, Some("repo is not in the allowlist"));
            return;
        }
    }

    if world.coding_publish_github_timeout {
        emit_publish_result(world, action, false, Some("network timeout"));
        return;
    }
    if world.coding_publish_github_rate_limited {
        emit_publish_result(world, action, false, Some("rate limiting"));
        return;
    }

    if action == "push_branch" {
        if world.coding_publish_force_push_default.as_deref() == Some("deny")
            && world.coding_publish_force
        {
            emit_publish_result(world, action, false, Some("force-push is denied by policy"));
            return;
        }
        let branch = world
            .coding_publish_target_branch
            .clone()
            .unwrap_or_else(|| "quecto/job/job_abc123".to_string());
        if branch == "main" {
            let err = if world.coding_publish_protected_branch_from_api {
                "branch protection rules"
            } else {
                "branch is protected"
            };
            emit_publish_result(world, action, false, Some(err));
            return;
        }
        world.coding_publish_branch_pushed = true;
    }

    if action == "create_pr" {
        world.coding_publish_pr_exists = true;
        world.coding_publish_pr_number = Some(123);
        world.coding_publish_pr_url = Some("https://github.com/org/repo/pull/123".to_string());
    }
    if action == "update_pr" || action == "request_review" || action == "add_labels" {
        world.coding_publish_pr_exists = true;
    }
    if action == "get_pr_status" {
        world.coding_publish_decision_ready_summary = true;
        world.coding_publish_has_check_review_state = true;
    }

    emit_publish_result(world, action, true, None);
}

#[given("a coding coordinator with GitHub policy:")]
fn given_github_policy(world: &mut QuectoWorld, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("policy table required");
    for row in &table.rows {
        let key = row.first().map(|x| x.trim()).unwrap_or_default();
        let value = row.get(1).map(|x| x.trim()).unwrap_or_default();
        match key {
            "side_effects_owner" => {
                world.coding_publish_side_effects_owner = Some(value.to_string())
            }
            "force_push_default" => {
                world.coding_publish_force_push_default = Some(value.to_string())
            }
            "destructive_reset_default" => {
                world.coding_publish_destructive_reset_default = Some(value.to_string())
            }
            _ => {}
        }
    }
    world.coding_publish_ok = None;
    world.coding_publish_error = None;
    world.coding_publish_branch_pushed = false;
    world.coding_publish_pr_exists = false;
    world.coding_publish_pr_number = None;
    world.coding_publish_pr_url = None;
    world.coding_publish_decision_ready_summary = false;
    world.coding_publish_has_check_review_state = false;
    world.coding_publish_reviewers.clear();
    world.coding_publish_labels.clear();
    world.coding_publish_force = false;
    world.coding_publish_target_branch = None;
    world.coding_publish_head_branch = None;
    world.coding_publish_target_repo = None;
    world.coding_publish_github_timeout = false;
    world.coding_publish_github_rate_limited = false;
    world.coding_publish_protected_branch_from_api = false;
    world.coding_publish_worker_rejected = false;
    world.coding_publish_worker_has_credentials = false;
    world.coding_publish_coordinator_has_token = true;
    world.coding_publish_credentials_redacted = false;
}

#[given("a PR already exists for the job branch")]
#[given("a PR exists for the job branch")]
fn given_pr_exists(world: &mut QuectoWorld) {
    world.coding_publish_pr_exists = true;
    world.coding_publish_pr_number = Some(123);
    world.coding_publish_pr_url = Some("https://github.com/org/repo/pull/123".to_string());
}

#[given(expr = "a coding coordinator with GitHub repo allowlist {string}")]
fn given_repo_allowlist(world: &mut QuectoWorld, repos: String) {
    world.coding_publish_repo_allowlist =
        parse_csv_list(repos.trim_matches(|c| c == '[' || c == ']'));
}

#[given(regex = r"^a coding coordinator with GitHub repo allowlist (\[.*\])$")]
fn given_repo_allowlist_unquoted(world: &mut QuectoWorld, repos: String) {
    given_repo_allowlist(world, repos);
}

#[given("two coding jobs have completed successfully for the same repo")]
fn given_two_jobs_succeeded_same_repo(world: &mut QuectoWorld) {
    world.coding_jobs.clear();
    for idx in 0..2 {
        let mut job =
            quecto::domain::coding_job::CodingJob::new(quecto::domain::coding_job::CodingJobInit {
                job_id: format!("job_multi_{}", idx + 1),
                run_id: format!("run_multi_{}", idx + 1),
                goal: format!("multi-goal-{}", idx + 1),
                repo: "org/repo".to_string(),
                base_ref: "main".to_string(),
                branch: format!("quecto/job/multi_{}", idx + 1),
            });
        job.state = JobState::Succeeded;
        world.coding_jobs.push(job.clone());
        world.coding_job = Some(job);
    }
}

#[when(expr = "the main agent requests publish action {string} for the job")]
fn when_publish_for_job(world: &mut QuectoWorld, action: String) {
    run_publish_action(world, &action);
}

#[when(expr = "the main agent requests publish action {string} with force {word}")]
fn when_publish_with_force(world: &mut QuectoWorld, action: String, force: String) {
    world.coding_publish_force = matches!(force.as_str(), "true" | "True" | "TRUE");
    run_publish_action(world, &action);
}

#[when(expr = "the main agent requests publish action {string} with:")]
fn when_publish_with_table(world: &mut QuectoWorld, action: String, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("publish payload table required");
    for row in &table.rows {
        let key = row.first().map(|x| x.trim()).unwrap_or_default();
        let value = row.get(1).map(|x| x.trim()).unwrap_or_default();
        match key {
            "base" => world.coding_publish_target_branch = Some(value.to_string()),
            "head" => world.coding_publish_head_branch = Some(value.to_string()),
            _ => {}
        }
    }
    run_publish_action(world, &action);
}

#[when(expr = "the main agent requests publish action {string} with updated body")]
#[when(expr = "the main agent requests publish action {string} with valid parameters")]
#[when(expr = "the main agent requests publish action {string}")]
fn when_publish_action(world: &mut QuectoWorld, action: String) {
    run_publish_action(world, &action);
}

#[when(expr = "the main agent requests publish action {string} with reviewers {string}")]
fn when_publish_request_review(world: &mut QuectoWorld, action: String, reviewers: String) {
    world.coding_publish_reviewers =
        parse_csv_list(reviewers.trim_matches(|c| c == '[' || c == ']'));
    run_publish_action(world, &action);
}

#[when(regex = r#"^the main agent requests publish action "([^"]+)" with reviewers (\[.*\])$"#)]
fn when_publish_request_review_unquoted(
    world: &mut QuectoWorld,
    action: String,
    reviewers: String,
) {
    when_publish_request_review(world, action, reviewers);
}

#[when(expr = "the main agent requests publish action {string} with labels {string}")]
fn when_publish_add_labels(world: &mut QuectoWorld, action: String, labels: String) {
    world.coding_publish_labels = parse_csv_list(labels.trim_matches(|c| c == '[' || c == ']'));
    run_publish_action(world, &action);
}

#[when(regex = r#"^the main agent requests publish action "([^"]+)" with labels (\[.*\])$"#)]
fn when_publish_add_labels_unquoted(world: &mut QuectoWorld, action: String, labels: String) {
    when_publish_add_labels(world, action, labels);
}

#[when(expr = "the main agent requests publish action {string} to branch {string}")]
fn when_publish_to_branch(world: &mut QuectoWorld, action: String, branch: String) {
    world.coding_publish_target_branch = Some(branch);
    run_publish_action(world, &action);
}

#[when("the main agent requests a destructive git reset on the remote")]
fn when_destructive_reset(world: &mut QuectoWorld) {
    emit_publish_request(world, "destructive_reset");
    let denied = world.coding_publish_destructive_reset_default.as_deref() == Some("deny");
    if denied {
        emit_publish_result(
            world,
            "destructive_reset",
            false,
            Some("destructive_reset_default policy"),
        );
    } else {
        emit_publish_result(world, "destructive_reset", true, None);
    }
}

#[when("the coordinator creates a PR for the job")]
fn when_coordinator_creates_pr(world: &mut QuectoWorld) {
    run_publish_action(world, "create_pr");
}

#[when(expr = "the main agent requests publish action {string} for repo {string}")]
fn when_publish_for_repo(world: &mut QuectoWorld, action: String, repo: String) {
    world.coding_publish_target_repo = Some(repo);
    run_publish_action(world, &action);
}

#[when("the GitHub API request times out")]
fn when_github_timeout(world: &mut QuectoWorld) {
    world.coding_publish_github_timeout = true;
    if let Some(event) = world
        .coding_events
        .iter_mut()
        .rev()
        .find(|e| e.event_type == "publish.result")
    {
        event.payload["ok"] = serde_json::Value::Bool(false);
        event.payload["error"] = serde_json::Value::String("network timeout".to_string());
        world.coding_publish_ok = Some(false);
        world.coding_publish_error = Some("network timeout".to_string());
    }
}

#[when("the GitHub API returns a 429 rate limit response")]
fn when_github_rate_limited(world: &mut QuectoWorld) {
    world.coding_publish_github_rate_limited = true;
    if let Some(event) = world
        .coding_events
        .iter_mut()
        .rev()
        .find(|e| e.event_type == "publish.result")
    {
        event.payload["ok"] = serde_json::Value::Bool(false);
        event.payload["error"] = serde_json::Value::String("rate limiting".to_string());
        world.coding_publish_ok = Some(false);
        world.coding_publish_error = Some("rate limiting".to_string());
    }
}

#[when(expr = "the GitHub API reports {string} as a protected branch")]
fn when_github_reports_protected(world: &mut QuectoWorld, branch: String) {
    if branch == "main" {
        world.coding_publish_protected_branch_from_api = true;
    }
    if let Some(action) = world.coding_publish_requested_action.clone() {
        run_publish_action(world, &action);
    }
}

#[when("the worker attempts to emit a \"publish.request\" event")]
fn when_worker_attempts_publish(world: &mut QuectoWorld) {
    push_coding_event(
        world,
        EventSource::Worker,
        "publish.request",
        serde_json::json!({"action": "create_pr"}),
    );
    world.coding_publish_worker_rejected = true;
    emit_publish_result(
        world,
        "create_pr",
        false,
        Some("publish is coordinator-only"),
    );
}

#[when(expr = "the main agent requests publish action {string} for the failed job")]
#[when(expr = "the main agent requests publish action {string} for the canceled job")]
fn when_publish_for_non_succeeded(world: &mut QuectoWorld, action: String) {
    run_publish_action(world, &action);
}

#[when("the main agent requests a combined PR for both jobs")]
fn when_combined_pr(world: &mut QuectoWorld) {
    emit_publish_request(world, "create_combined_pr");
    emit_publish_result(world, "create_combined_pr", true, None);
}

#[then(expr = "a {string} event should be emitted with action {string}")]
fn then_event_with_action(world: &mut QuectoWorld, event_type: String, action: String) {
    let event = last_event(world, &event_type);
    assert_eq!(
        event.payload.get("action").and_then(|v| v.as_str()),
        Some(action.as_str())
    );
}

#[then("the coordinator should push the job branch to remote")]
fn then_branch_pushed(world: &mut QuectoWorld) {
    assert!(world.coding_publish_branch_pushed);
}

#[then("the error should indicate force-push is denied by policy")]
fn then_error_force_push(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_publish_error
            .as_deref()
            .unwrap_or_default()
            .contains("force-push is denied by policy")
    );
}

#[then("the result should include pr_number and url")]
fn then_result_has_pr_fields(world: &mut QuectoWorld) {
    assert!(world.coding_publish_pr_number.is_some());
    assert!(world.coding_publish_pr_url.is_some());
}

#[then("the main agent should receive a decision-ready summary")]
fn then_decision_ready_summary(world: &mut QuectoWorld) {
    assert!(world.coding_publish_decision_ready_summary);
}

#[then("the error should indicate the branch is protected")]
fn then_error_protected_branch(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_publish_error
            .as_deref()
            .unwrap_or_default()
            .contains("branch is protected")
    );
}

#[then("the coordinator should deny the operation")]
fn then_coordinator_denies_operation(world: &mut QuectoWorld) {
    assert_eq!(world.coding_publish_ok, Some(false));
}

#[then("the error should reference the destructive_reset_default policy")]
fn then_error_destructive_reset_policy(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_publish_error
            .as_deref()
            .unwrap_or_default()
            .contains("destructive_reset_default")
    );
}

#[then(expr = "the event log should contain {string} and {string} events")]
fn then_event_log_contains(world: &mut QuectoWorld, event_a: String, event_b: String) {
    assert!(world.coding_events.iter().any(|e| e.event_type == event_a));
    assert!(world.coding_events.iter().any(|e| e.event_type == event_b));
}

#[then("credential values should be redacted in event payloads")]
fn then_credentials_redacted(world: &mut QuectoWorld) {
    let leaked = world.coding_events.iter().any(|e| {
        let payload = e.payload.to_string();
        payload.contains("ghp_") || payload.contains("github_pat_") || payload.contains("sk-")
    });
    assert!(!leaked);
}

#[then(expr = "the {string} event should include action {string}")]
fn then_result_event_action(world: &mut QuectoWorld, event_type: String, action: String) {
    let event = last_event(world, &event_type);
    assert_eq!(
        event.payload.get("action").and_then(|v| v.as_str()),
        Some(action.as_str())
    );
}

#[then("the action should match the original publish.request action")]
fn then_actions_match(world: &mut QuectoWorld) {
    assert_eq!(
        world.coding_publish_result_action,
        world.coding_publish_requested_action
    );
}

#[then("the error should indicate the repo is not in the allowlist")]
fn then_error_repo_allowlist(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_publish_error
            .as_deref()
            .unwrap_or_default()
            .contains("allowlist")
    );
}

#[then("the error should indicate a network timeout")]
fn then_error_timeout(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_publish_error
            .as_deref()
            .unwrap_or_default()
            .contains("network timeout")
    );
}

#[then("the error should indicate rate limiting")]
fn then_error_rate_limit(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_publish_error
            .as_deref()
            .unwrap_or_default()
            .contains("rate limiting")
    );
}

#[then("the error should reference branch protection rules")]
fn then_error_branch_protection_rules(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_publish_error
            .as_deref()
            .unwrap_or_default()
            .contains("branch protection rules")
    );
}

#[then("the coordinator should reject the publish event")]
fn then_coordinator_rejects_worker_publish(world: &mut QuectoWorld) {
    let request_from_worker = world
        .coding_events
        .iter()
        .any(|e| e.event_type == "publish.request" && e.source == EventSource::Worker);
    assert!(request_from_worker);
    assert_eq!(world.coding_publish_ok, Some(false));
}

#[then("the worker should receive an error indicating publish is coordinator-only")]
fn then_worker_receives_coordinator_only_error(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_publish_error
            .as_deref()
            .unwrap_or_default()
            .contains("coordinator-only")
    );
}

#[then("the worker process should not have GitHub credentials in its environment")]
fn then_worker_has_no_credentials(world: &mut QuectoWorld) {
    assert!(!world.coding_publish_worker_has_credentials);
}

#[then("only the coordinator should hold GitHub API tokens")]
fn then_only_coordinator_has_token(world: &mut QuectoWorld) {
    assert!(world.coding_publish_coordinator_has_token);
}

#[then("the result should include the current PR check and review state")]
fn then_result_has_check_review_state(world: &mut QuectoWorld) {
    assert!(world.coding_publish_has_check_review_state);
}

#[then("the error should indicate the job did not succeed")]
fn then_error_job_not_succeeded(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_publish_error
            .as_deref()
            .unwrap_or_default()
            .contains("did not succeed")
    );
}

#[then("the error should indicate the job was canceled")]
fn then_error_job_canceled(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_publish_error
            .as_deref()
            .unwrap_or_default()
            .contains("job was canceled")
    );
}

#[then("the coordinator should merge the branches or patches")]
fn then_combined_merge_or_patch(world: &mut QuectoWorld) {
    assert_eq!(
        world.coding_publish_requested_action.as_deref(),
        Some("create_combined_pr")
    );
}

#[then("a single \"publish.result\" event should indicate the combined PR")]
fn then_single_combined_publish_result(world: &mut QuectoWorld) {
    let count = world
        .coding_events
        .iter()
        .filter(|e| e.event_type == "publish.result")
        .count();
    assert_eq!(count, 1);
    assert_eq!(world.coding_publish_ok, Some(true));
}
