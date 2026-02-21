use super::*;

// Heartbeat Steps
// ===========================================================================

#[given(expr = "a HEARTBEAT.md with content:")]
fn given_heartbeat_content(world: &mut QuectoWorld, step: &gherkin::Step) {
    let content = step.docstring().expect("step should have a docstring");
    world.heartbeat_content = Some(content.to_string());
}

#[given(expr = "a workspace with a HEARTBEAT.md file containing:")]
fn given_workspace_with_heartbeat(world: &mut QuectoWorld, step: &gherkin::Step) {
    let content = step.docstring().expect("step should have a docstring");
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    std::fs::write(ws.join("HEARTBEAT.md"), content).expect("write HEARTBEAT.md");
    world.heartbeat_workspace = Some(ws);
    world._temp_dir = Some(td);
}

#[given("a workspace without a HEARTBEAT.md file")]
fn given_workspace_without_heartbeat(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    world.heartbeat_workspace = Some(td.path().to_path_buf());
    world._temp_dir = Some(td);
}

#[given(expr = "a heartbeat result with {int} tasks found, {int} executed, and ok {word}")]
fn given_heartbeat_result(world: &mut QuectoWorld, found: usize, executed: usize, ok: String) {
    world.heartbeat_result = Some(HeartbeatResult {
        tasks_found: found,
        tasks_executed: executed,
        ok: ok == "true",
    });
}

#[when("the heartbeat content is parsed")]
fn when_heartbeat_parsed(world: &mut QuectoWorld) {
    let content = world
        .heartbeat_content
        .as_ref()
        .expect("heartbeat content not set");
    world.heartbeat_tasks = Some(heartbeat::parse_heartbeat(content));
}

#[when("the heartbeat loads tasks from the workspace")]
fn when_heartbeat_loads_tasks(world: &mut QuectoWorld) {
    let ws = world
        .heartbeat_workspace
        .as_ref()
        .expect("heartbeat workspace not set")
        .clone();
    let tasks = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(heartbeat::load_tasks(&ws))
        .unwrap();
    world.heartbeat_tasks = Some(tasks);
}

#[then(expr = "the parsed tasks should contain {int} items")]
fn then_parsed_tasks_count(world: &mut QuectoWorld, expected: usize) {
    let tasks = world.heartbeat_tasks.as_ref().expect("no parsed tasks");
    assert_eq!(
        tasks.len(),
        expected,
        "expected {} tasks, got {}",
        expected,
        tasks.len()
    );
}

#[then(expr = "task {int} should be {string}")]
fn then_task_message(world: &mut QuectoWorld, index: usize, expected: String) {
    let tasks = world.heartbeat_tasks.as_ref().expect("no parsed tasks");
    let task = &tasks[index - 1]; // 1-indexed
    assert_eq!(
        task.message, expected,
        "expected task {} to be '{}', got '{}'",
        index, expected, task.message
    );
}

#[then("no tasks should be marked as spawn")]
fn then_no_spawn_tasks(world: &mut QuectoWorld) {
    let tasks = world.heartbeat_tasks.as_ref().expect("no parsed tasks");
    assert!(
        tasks.iter().all(|t| !t.use_spawn),
        "expected no spawn tasks, but some are marked as spawn"
    );
}

#[then(expr = "task {int} should be marked as spawn")]
fn then_task_is_spawn(world: &mut QuectoWorld, index: usize) {
    let tasks = world.heartbeat_tasks.as_ref().expect("no parsed tasks");
    let task = &tasks[index - 1];
    assert!(
        task.use_spawn,
        "expected task {} to be marked as spawn",
        index
    );
}

#[then(expr = "task {int} should not be marked as spawn")]
fn then_task_not_spawn(world: &mut QuectoWorld, index: usize) {
    let tasks = world.heartbeat_tasks.as_ref().expect("no parsed tasks");
    let task = &tasks[index - 1];
    assert!(
        !task.use_spawn,
        "expected task {} to NOT be marked as spawn",
        index
    );
}

#[then(expr = "the heartbeat status should be {string}")]
fn then_heartbeat_status(world: &mut QuectoWorld, expected: String) {
    let result = world
        .heartbeat_result
        .as_ref()
        .expect("no heartbeat result");
    assert_eq!(
        result.status(),
        expected,
        "expected status '{}', got '{}'",
        expected,
        result.status()
    );
}

// ===========================================================================
// Gateway heartbeat scenario steps
// ===========================================================================

#[given(expr = "a workspace HEARTBEAT.md containing:")]
fn given_workspace_heartbeat_md(world: &mut QuectoWorld, step: &gherkin::Step) {
    let content = step.docstring().expect("step should have a docstring");
    let ws = world.heartbeat_workspace.as_ref().expect(
        "heartbeat workspace not set (run 'a running gateway with a mock LLM provider' first)",
    );
    std::fs::write(ws.join("HEARTBEAT.md"), content).expect("write HEARTBEAT.md");
}

#[given("no HEARTBEAT.md in the workspace")]
fn given_no_heartbeat_md(world: &mut QuectoWorld) {
    let ws = world
        .heartbeat_workspace
        .as_ref()
        .expect("heartbeat workspace not set");
    let path = ws.join("HEARTBEAT.md");
    if path.exists() {
        std::fs::remove_file(&path).expect("remove HEARTBEAT.md");
    }
}

#[given(expr = "the config has heartbeat_interval_minutes {int}")]
fn given_config_heartbeat_interval(world: &mut QuectoWorld, minutes: u32) {
    let config = world
        .gateway_tick_config
        .get_or_insert_with(Config::default);
    config.heartbeat.interval = minutes;
}

#[when("the heartbeat tick fires")]
fn when_heartbeat_tick_fires(world: &mut QuectoWorld) {
    let agent = &world
        ._gateway_mock_agent
        .as_ref()
        .expect("mock agent not set")
        .0;
    let ws = world
        .heartbeat_workspace
        .as_ref()
        .expect("heartbeat workspace not set")
        .clone();

    let results = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(heartbeat::execute_heartbeat_tick(
            &ws,
            agent.as_ref(),
            std::time::Duration::from_secs(60),
        ))
        .unwrap();
    world.heartbeat_tick_results = Some(results);
}

#[then(expr = "the task {string} should be dispatched via the spawn tool")]
fn then_task_dispatched_via_spawn(world: &mut QuectoWorld, task_msg: String) {
    let results = world
        .heartbeat_tick_results
        .as_ref()
        .expect("no heartbeat tick results");
    let found = results
        .iter()
        .any(|r| r.message == task_msg && r.dispatched_via_spawn);
    assert!(
        found,
        "expected task '{}' dispatched via spawn, got: {:?}",
        task_msg,
        results
            .iter()
            .map(|r| (&r.message, r.dispatched_via_spawn))
            .collect::<Vec<_>>()
    );
}

// ===========================================================================
