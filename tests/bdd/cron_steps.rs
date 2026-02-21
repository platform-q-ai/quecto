use super::*;

// Cron Steps
// ===========================================================================

fn ensure_cron_store(world: &mut QuectoWorld) {
    if world.cron_store.is_none() {
        if world._temp_dir.is_none() {
            let td = TempDir::new().expect("failed to create temp dir");
            world._temp_dir = Some(td);
        }
        let base = world._temp_dir.as_ref().unwrap().path().to_path_buf();
        world.cron_workspace = Some(base.clone());
        world.cron_store = Some(FileCronStore::new(base));
    }
}

fn make_interval_job(name: &str, seconds: u64) -> CronJob {
    CronJob {
        id: name.to_lowercase().replace(' ', "-"),
        name: name.to_string(),
        message: format!("Run {}", name),
        schedule: CronSchedule::Interval { seconds },
        enabled: true,
        deliver_to: None,
        last_error: None,
        last_run_at: 0,
    }
}

fn make_cron_expr_job(name: &str, expr: &str) -> CronJob {
    CronJob {
        id: name.to_lowercase().replace(' ', "-"),
        name: name.to_string(),
        message: format!("Run {}", name),
        schedule: CronSchedule::Cron {
            expression: expr.to_string(),
        },
        enabled: true,
        deliver_to: None,
        last_error: None,
        last_run_at: 0,
    }
}

#[given("a cron store")]
fn given_cron_store(world: &mut QuectoWorld) {
    ensure_cron_store(world);
}

#[given(expr = "a job {string} with interval {int} seconds exists")]
fn given_job_with_interval(world: &mut QuectoWorld, name: String, seconds: u64) {
    ensure_cron_store(world);
    let store = world.cron_store.as_ref().unwrap();
    store.add(make_interval_job(&name, seconds)).unwrap();
}

#[given(expr = "a job {string} with cron expression {string} exists")]
fn given_job_with_cron_expr(world: &mut QuectoWorld, name: String, expr: String) {
    ensure_cron_store(world);
    let store = world.cron_store.as_ref().unwrap();
    store.add(make_cron_expr_job(&name, &expr)).unwrap();
}

#[given(expr = "a disabled job {string} with interval {int} seconds exists")]
fn given_disabled_job(world: &mut QuectoWorld, name: String, seconds: u64) {
    ensure_cron_store(world);
    let store = world.cron_store.as_ref().unwrap();
    let mut job = make_interval_job(&name, seconds);
    job.enabled = false;
    store.add(job).unwrap();
}

#[when(expr = "I add a job {string} with interval {int} seconds")]
fn when_add_interval_job(world: &mut QuectoWorld, name: String, seconds: u64) {
    let store = world.cron_store.as_ref().unwrap();
    store.add(make_interval_job(&name, seconds)).unwrap();
}

#[when(expr = "I add a job {string} with cron expression {string}")]
fn when_add_cron_expr_job(world: &mut QuectoWorld, name: String, expr: String) {
    let store = world.cron_store.as_ref().unwrap();
    store.add(make_cron_expr_job(&name, &expr)).unwrap();
}

#[when("I list all jobs")]
fn when_list_jobs(world: &mut QuectoWorld) {
    let store = world.cron_store.as_ref().unwrap();
    world.cron_jobs = Some(store.list().unwrap());
}

#[when(expr = "I remove the job {string}")]
fn when_remove_job(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = store
        .find_by_name(&name)
        .unwrap()
        .unwrap_or_else(|| panic!("job '{}' not found", name));
    store.remove(&job.id).unwrap();
}

#[when(expr = "I disable the job {string}")]
fn when_disable_job(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = store
        .find_by_name(&name)
        .unwrap()
        .unwrap_or_else(|| panic!("job '{}' not found", name));
    store.set_enabled(&job.id, false).unwrap();
}

#[when(expr = "I enable the job {string}")]
fn when_enable_job(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = store
        .find_by_name(&name)
        .unwrap()
        .unwrap_or_else(|| panic!("job '{}' not found", name));
    store.set_enabled(&job.id, true).unwrap();
}

#[when("the cron store is recreated from the same directory")]
fn when_cron_store_recreated(world: &mut QuectoWorld) {
    let ws = world.cron_workspace.as_ref().unwrap().clone();
    world.cron_store = Some(FileCronStore::new(ws));
}

#[then(expr = "the job {string} should exist in the store")]
fn then_job_exists(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let found = store.find_by_name(&name).unwrap();
    assert!(found.is_some(), "job '{}' should exist", name);
}

#[then(expr = "the job {string} should not exist in the store")]
fn then_job_not_exists(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let found = store.find_by_name(&name).unwrap();
    assert!(found.is_none(), "job '{}' should not exist", name);
}

#[then("the job should be enabled")]
fn then_job_enabled(world: &mut QuectoWorld) {
    let store = world.cron_store.as_ref().unwrap();
    let jobs = store.list().unwrap();
    let last = jobs.last().expect("no jobs");
    assert!(last.enabled, "job should be enabled");
}

#[then(expr = "the job {string} should be disabled")]
fn then_job_disabled(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = store.find_by_name(&name).unwrap().unwrap();
    assert!(!job.enabled, "job '{}' should be disabled", name);
}

#[then(expr = "the job {string} should be enabled")]
fn then_named_job_enabled(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = store.find_by_name(&name).unwrap().unwrap();
    assert!(job.enabled, "job '{}' should be enabled", name);
}

#[then(expr = "the job list should contain {int} jobs")]
fn then_job_list_count(world: &mut QuectoWorld, expected: usize) {
    let jobs = world.cron_jobs.as_ref().expect("no job list");
    assert_eq!(
        jobs.len(),
        expected,
        "expected {} jobs, got {}",
        expected,
        jobs.len()
    );
}

#[then(expr = "the job list should include {string}")]
fn then_job_list_includes(world: &mut QuectoWorld, name: String) {
    let jobs = world.cron_jobs.as_ref().expect("no job list");
    assert!(
        jobs.iter().any(|j| j.name == name),
        "job list should include '{}', has: {:?}",
        name,
        jobs.iter().map(|j| &j.name).collect::<Vec<_>>()
    );
}

// ===========================================================================
// Gateway cron scenario steps
// ===========================================================================

#[given(expr = "a cron job {string} with interval {int} seconds and message {string}")]
fn given_gateway_cron_job_with_message(
    world: &mut QuectoWorld,
    name: String,
    seconds: u64,
    message: String,
) {
    let store = world
        .gateway_cron_store
        .as_ref()
        .expect("gateway cron store not set");
    let job = CronJob {
        id: name.to_lowercase().replace(' ', "-"),
        name: name.clone(),
        message,
        schedule: CronSchedule::Interval { seconds },
        enabled: true,
        deliver_to: None,
        last_error: None,
        last_run_at: 0,
    };
    store.add(job).unwrap();
}

#[given(expr = "a disabled cron job {string} with interval {int} seconds")]
fn given_gateway_disabled_cron_job(world: &mut QuectoWorld, name: String, seconds: u64) {
    let store = world
        .gateway_cron_store
        .as_ref()
        .expect("gateway cron store not set");
    let job = CronJob {
        id: name.to_lowercase().replace(' ', "-"),
        name: name.clone(),
        message: format!("Run {}", name),
        schedule: CronSchedule::Interval { seconds },
        enabled: false,
        deliver_to: None,
        last_error: None,
        last_run_at: 0,
    };
    store.add(job).unwrap();
}

#[given(expr = "the config has exec_timeout_minutes {int}")]
fn given_config_exec_timeout(world: &mut QuectoWorld, minutes: u32) {
    let config = world
        .gateway_tick_config
        .get_or_insert_with(Config::default);
    config.tools.cron.exec_timeout_minutes = minutes;
}

#[given("a mock Telegram API")]
fn given_mock_telegram_api(_world: &mut QuectoWorld) {
    // No-op: we test delivery via the CronJobResult.deliver_to field
    // rather than actually calling the Telegram API. The "Then" step
    // inspects the cron tick results.
}

#[given(expr = "a cron job {string} with interval {int} seconds and deliver_to {string}")]
fn given_gateway_cron_job_deliver_to(
    world: &mut QuectoWorld,
    name: String,
    seconds: u64,
    deliver_to: String,
) {
    let store = world
        .gateway_cron_store
        .as_ref()
        .expect("gateway cron store not set");
    let job = CronJob {
        id: name.to_lowercase().replace(' ', "-"),
        name: name.clone(),
        message: format!("Run {}", name),
        schedule: CronSchedule::Interval { seconds },
        enabled: true,
        deliver_to: Some(deliver_to),
        last_error: None,
        last_run_at: 0,
    };
    store.add(job).unwrap();
}

#[when("the cron tick fires")]
fn when_cron_tick_fires(world: &mut QuectoWorld) {
    let agent = &world
        ._gateway_mock_agent
        .as_ref()
        .expect("mock agent not set")
        .0;
    let store = world
        .gateway_cron_store
        .as_ref()
        .expect("cron store not set");
    let config = world
        .gateway_tick_config
        .as_ref()
        .cloned()
        .unwrap_or_default();
    let timeout =
        std::time::Duration::from_secs(u64::from(config.tools.cron.exec_timeout_minutes) * 60);

    let results = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(cron_executor::execute_cron_tick(
            store.as_ref(),
            agent.as_ref(),
            timeout,
        ))
        .unwrap();
    world.cron_tick_results = Some(results);
}

#[when("the cron job starts executing and exceeds the timeout")]
fn when_cron_job_exceeds_timeout(world: &mut QuectoWorld) {
    let store = world
        .gateway_cron_store
        .as_ref()
        .expect("cron store not set");
    // Use a very short timeout so the slow agent always times out.
    let slow_agent = SlowMockAgent;
    let timeout = std::time::Duration::from_millis(50);

    let results = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(cron_executor::execute_cron_tick(
            store.as_ref(),
            &slow_agent,
            timeout,
        ))
        .unwrap();
    world.cron_tick_results = Some(results);
}

#[when(expr = "the cron tick fires for job {string}")]
fn when_cron_tick_fires_for_job(world: &mut QuectoWorld, _name: String) {
    // Same as "the cron tick fires" — all enabled jobs run.
    when_cron_tick_fires(world);
}

#[then("the job execution should be terminated")]
fn then_job_execution_terminated(world: &mut QuectoWorld) {
    let results = world
        .cron_tick_results
        .as_ref()
        .expect("no cron tick results");
    assert!(
        results.iter().any(|r| !r.ok),
        "expected at least one failed job, got: {:?}",
        results
    );
}

#[then(expr = "the job should be marked as last_error containing {string}")]
fn then_job_last_error_contains(world: &mut QuectoWorld, expected: String) {
    let store = world
        .gateway_cron_store
        .as_ref()
        .expect("cron store not set");
    let jobs = store.list().unwrap();
    let has_error = jobs
        .iter()
        .any(|j| j.last_error.as_ref().is_some_and(|e| e.contains(&expected)));
    assert!(
        has_error,
        "expected a job with last_error containing '{}', got: {:?}",
        expected,
        jobs.iter()
            .map(|j| (&j.name, &j.last_error))
            .collect::<Vec<_>>()
    );
}

#[then(expr = "the Telegram API should receive a sendMessage to chat {string}")]
fn then_telegram_receives_send_message(world: &mut QuectoWorld, chat_id: String) {
    let results = world
        .cron_tick_results
        .as_ref()
        .expect("no cron tick results");
    let deliver_target = format!("telegram:{}", chat_id);
    assert!(
        results
            .iter()
            .any(|r| r.deliver_to.as_deref() == Some(&deliver_target)),
        "expected result with deliver_to '{}', got: {:?}",
        deliver_target,
        results.iter().map(|r| &r.deliver_to).collect::<Vec<_>>()
    );
}

#[then(expr = "the message should contain {string}")]
fn then_message_contains(world: &mut QuectoWorld, expected: String) {
    let results = world
        .cron_tick_results
        .as_ref()
        .expect("no cron tick results");
    assert!(
        results.iter().any(|r| r.response.contains(&expected)),
        "expected a result containing '{}', got: {:?}",
        expected,
        results.iter().map(|r| &r.response).collect::<Vec<_>>()
    );
}

// ===========================================================================
