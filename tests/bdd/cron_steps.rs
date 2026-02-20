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
    let job = cron_store::find_by_name(store, &name)
        .unwrap()
        .unwrap_or_else(|| panic!("job '{}' not found", name));
    store.remove(&job.id).unwrap();
}

#[when(expr = "I disable the job {string}")]
fn when_disable_job(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = cron_store::find_by_name(store, &name)
        .unwrap()
        .unwrap_or_else(|| panic!("job '{}' not found", name));
    store.set_enabled(&job.id, false).unwrap();
}

#[when(expr = "I enable the job {string}")]
fn when_enable_job(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = cron_store::find_by_name(store, &name)
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
    let found = cron_store::find_by_name(store, &name).unwrap();
    assert!(found.is_some(), "job '{}' should exist", name);
}

#[then(expr = "the job {string} should not exist in the store")]
fn then_job_not_exists(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let found = cron_store::find_by_name(store, &name).unwrap();
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
    let job = cron_store::find_by_name(store, &name).unwrap().unwrap();
    assert!(!job.enabled, "job '{}' should be disabled", name);
}

#[then(expr = "the job {string} should be enabled")]
fn then_named_job_enabled(world: &mut QuectoWorld, name: String) {
    let store = world.cron_store.as_ref().unwrap();
    let job = cron_store::find_by_name(store, &name).unwrap().unwrap();
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
