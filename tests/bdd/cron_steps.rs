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
        run_once: false,
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
        run_once: false,
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
        run_once: false,
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
        run_once: false,
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
        run_once: false,
    };
    store.add(job).unwrap();
}

#[given(expr = "a cron job {string} with cron expression {string} and message {string}")]
fn given_gateway_cron_job_with_expression_and_message(
    world: &mut QuectoWorld,
    name: String,
    expression: String,
    message: String,
) {
    let store = world
        .gateway_cron_store
        .as_ref()
        .expect("gateway cron store not set");
    let job = CronJob {
        id: name.to_lowercase().replace(' ', "-"),
        name,
        message,
        schedule: CronSchedule::Cron { expression },
        enabled: true,
        deliver_to: None,
        last_error: None,
        last_run_at: 0,
        run_once: false,
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
// Issue #105: Run-once cron jobs
// ===========================================================================

#[when(expr = "I add a run-once job {string} with interval {int} seconds and message {string}")]
fn when_add_run_once_job(world: &mut QuectoWorld, name: String, seconds: u64, message: String) {
    ensure_cron_store(world);
    let store = world.cron_store.as_ref().unwrap();
    let mut job = make_interval_job(&name, seconds);
    job.message = message;
    job.run_once = true;
    store.add(job).unwrap();
}

#[given(expr = "a run-once job {string} with interval {int} seconds exists")]
fn given_run_once_job_exists(world: &mut QuectoWorld, name: String, seconds: u64) {
    ensure_cron_store(world);
    let store = world.cron_store.as_ref().unwrap();
    let mut job = make_interval_job(&name, seconds);
    job.run_once = true;
    store.add(job).unwrap();
}

#[given(expr = "a run-once cron job {string} with interval {int} seconds and message {string}")]
fn given_gateway_run_once_cron_job(
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
        run_once: true,
    };
    store.add(job).unwrap();
}

#[then(expr = "the job {string} should be marked as run_once")]
fn then_job_is_run_once(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = store
        .find_by_name(&name)
        .unwrap()
        .unwrap_or_else(|| panic!("job '{}' not found", name));
    assert!(job.run_once, "job '{}' should be marked as run_once", name);
}

#[then(expr = "the gateway job {string} should not exist in the store")]
fn then_gateway_job_not_exists(world: &mut QuectoWorld, name: String) {
    let store = world
        .gateway_cron_store
        .as_ref()
        .expect("gateway cron store not set");
    let found = store.find_by_name(&name).unwrap();
    assert!(
        found.is_none(),
        "gateway job '{}' should not exist after run_once execution",
        name
    );
}

#[then(expr = "the gateway job {string} should still exist in the store")]
fn then_gateway_job_still_exists(world: &mut QuectoWorld, name: String) {
    let store = world
        .gateway_cron_store
        .as_ref()
        .expect("gateway cron store not set");
    let found = store.find_by_name(&name).unwrap();
    assert!(found.is_some(), "gateway job '{}' should still exist", name);
}

#[when("I list all jobs via the cron tool")]
fn when_list_jobs_via_tool(world: &mut QuectoWorld) {
    ensure_cron_store(world);
    let tool = CronTool::new(Arc::new(FileCronStore::new(
        world.cron_workspace.as_ref().unwrap(),
    )));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(r#"{"action":"list"}"#)).unwrap();
    world.cron_tool_list_output = Some(result.content);
}

#[then(expr = "the list output should contain {string}")]
fn then_list_output_contains(world: &mut QuectoWorld, expected: String) {
    let output = world
        .cron_tool_list_output
        .as_ref()
        .expect("no cron tool list output");
    assert!(
        output.contains(&expected),
        "expected list output to contain '{}', got: {}",
        expected,
        output
    );
}

// ===========================================================================
// Issue #106: Cron job result delivery
// ===========================================================================

#[given("a running gateway with a mock LLM provider and outbound channel")]
fn given_gateway_with_outbound_channel(world: &mut QuectoWorld) {
    // Set up standard gateway mock context (same as gateway_steps)
    let messages = Arc::new(Mutex::new(Vec::new()));
    world.mock_agent_messages = messages.clone();
    let agent = RecordingMockAgent {
        response: "OK".to_string(),
        messages,
    };
    world._gateway_mock_agent = Some(DebugAgent(Arc::new(agent)));
    world.gateway_cron_store = Some(Arc::new(InMemoryCronStore::new()));
    world.gateway_tick_config = Some(Config::default());
    // Create outbound channel for capturing messages
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    world.cron_outbound_tx = Some(tx);
    world.cron_outbound_rx = Some(rx);
}

struct DeliverToCronSpec {
    name: String,
    seconds: u64,
    message: String,
    deliver_to: String,
}

fn add_cron_job_with_deliver_to(world: &mut QuectoWorld, spec: DeliverToCronSpec) {
    let store = world
        .gateway_cron_store
        .as_ref()
        .expect("gateway cron store not set");
    let job = CronJob {
        id: spec.name.to_lowercase().replace(' ', "-"),
        name: spec.name.clone(),
        message: spec.message,
        schedule: CronSchedule::Interval {
            seconds: spec.seconds,
        },
        enabled: true,
        deliver_to: Some(spec.deliver_to),
        last_error: None,
        last_run_at: 0,
        run_once: false,
    };
    store.add(job).unwrap();
}

#[given(
    expr = "a cron job {string} with interval {int} seconds and message {string} and deliver_to {string}"
)]
// Cucumber step functions have unavoidable arg count: world + all captures from the step expression.
#[allow(clippy::too_many_arguments)]
fn given_cron_job_with_message_and_deliver_to(
    world: &mut QuectoWorld,
    name: String,
    seconds: u64,
    message: String,
    deliver_to: String,
) {
    add_cron_job_with_deliver_to(
        world,
        DeliverToCronSpec {
            name,
            seconds,
            message,
            deliver_to,
        },
    );
}

#[when("the cron tick fires and results are delivered")]
fn when_cron_tick_fires_and_delivers(world: &mut QuectoWorld) {
    let agent = &world
        ._gateway_mock_agent
        .as_ref()
        .expect("mock agent not set")
        .0;
    let store = world
        .gateway_cron_store
        .as_ref()
        .expect("cron store not set");
    let timeout = std::time::Duration::from_secs(60);
    let outbound_tx = world
        .cron_outbound_tx
        .as_ref()
        .expect("outbound_tx not set")
        .clone();
    let default_send_to = world.cron_default_send_to.clone();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let results = rt
        .block_on(cron_executor::execute_cron_tick(
            store.as_ref(),
            agent.as_ref(),
            timeout,
        ))
        .unwrap();

    // Simulate gateway delivery logic (mirrors deliver_cron_result in services.rs)
    // Uses default_send_to as fallback when job has no deliver_to — Issue #193.
    for result in &results {
        if result.ok {
            let target = result.deliver_to.as_deref().or(default_send_to.as_deref());
            if let Some(target) = target {
                let msg = OutboundMessage {
                    target: target.to_string(),
                    text: result.response.clone(),
                };
                rt.block_on(outbound_tx.send(msg))
                    .expect("outbound send failed");
            }
        }
    }

    world.cron_tick_results = Some(results);
}

#[then(expr = "the outbound channel should have received a message to {string}")]
fn then_outbound_received_message_to(world: &mut QuectoWorld, target: String) {
    let rx = world
        .cron_outbound_rx
        .as_mut()
        .expect("outbound_rx not set");
    let mut found = false;
    while let Ok(msg) = rx.try_recv() {
        if msg.target == target {
            found = true;
            world.last_outbound_message = Some(msg);
        }
    }
    assert!(
        found,
        "expected outbound message to '{}', but none received",
        target
    );
}

#[then(expr = "the outbound message should contain {string}")]
fn then_outbound_message_contains(world: &mut QuectoWorld, expected: String) {
    let msg = world
        .last_outbound_message
        .as_ref()
        .expect("no outbound message captured");
    assert!(
        msg.text.contains(&expected),
        "expected outbound message to contain '{}', got: {}",
        expected,
        msg.text
    );
}

#[then("the outbound channel should not have received any messages")]
fn then_outbound_no_messages(world: &mut QuectoWorld) {
    let rx = world
        .cron_outbound_rx
        .as_mut()
        .expect("outbound_rx not set");
    let msg = rx.try_recv();
    assert!(
        msg.is_err(),
        "expected no outbound messages, but received one"
    );
}

// ===========================================================================
// Issue #193: default_send_to fallback for cron jobs
// ===========================================================================

#[given(expr = "the gateway is configured with default_send_to {string}")]
fn given_gateway_default_send_to(world: &mut QuectoWorld, target: String) {
    world.cron_default_send_to = Some(target);
}

#[given(
    expr = "a cron job {string} with interval {int} seconds and message {string} and no deliver_to"
)]
fn given_cron_job_no_deliver_to(
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
        name,
        message,
        schedule: CronSchedule::Interval { seconds },
        enabled: true,
        deliver_to: None,
        last_error: None,
        last_run_at: 0,
        run_once: false,
    };
    store.add(job).unwrap();
}
