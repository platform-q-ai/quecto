//! BDD step definitions for coordinator spawn and liveness scenarios.
//!
//! These steps test the CoordinatorSpawner domain port trait and the
//! integration of auto-spawn into the CoordinatorDelegationTool.

use cucumber::{given, then, when};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use quecto::domain::coding_ipc::{CoordinatorSpawner, SpawnResult};
use quecto::infrastructure::coding::coordinator_spawner::{
    CoordinatorProcessSpawner, CoordinatorSpawnConfig,
};
use quecto::infrastructure::tools::coding_delegation::CoordinatorDelegationTool;

use crate::{BddDelegMockIpc, QuectoWorld};

// ============================================================================
// Mock Spawner for BDD testing
// ============================================================================

/// BDD mock spawner that records calls and returns configurable results.
#[derive(Debug)]
pub(crate) struct BddMockSpawner {
    pub alive: bool,
    pub existing_pid: u32,
    pub spawned_pid: u32,
    pub fail: bool,
    pub ensure_alive_calls: Mutex<u32>,
    pub spawn_launches: Mutex<u32>,
}

impl BddMockSpawner {
    fn already_alive(pid: u32) -> Self {
        Self {
            alive: true,
            existing_pid: pid,
            spawned_pid: 0,
            fail: false,
            ensure_alive_calls: Mutex::new(0),
            spawn_launches: Mutex::new(0),
        }
    }

    fn needs_spawn(new_pid: u32) -> Self {
        Self {
            alive: false,
            existing_pid: 0,
            spawned_pid: new_pid,
            fail: false,
            ensure_alive_calls: Mutex::new(0),
            spawn_launches: Mutex::new(0),
        }
    }

    fn failing() -> Self {
        Self {
            alive: false,
            existing_pid: 0,
            spawned_pid: 0,
            fail: true,
            ensure_alive_calls: Mutex::new(0),
            spawn_launches: Mutex::new(0),
        }
    }
}

impl CoordinatorSpawner for BddMockSpawner {
    fn ensure_alive(&self) -> Result<SpawnResult, String> {
        *self.ensure_alive_calls.lock().unwrap() += 1;
        if self.fail {
            return Err("spawn failed: mock".to_string());
        }
        if self.alive {
            Ok(SpawnResult {
                pid: self.existing_pid,
                spawned: false,
            })
        } else {
            *self.spawn_launches.lock().unwrap() += 1;
            Ok(SpawnResult {
                pid: self.spawned_pid,
                spawned: true,
            })
        }
    }
}

// ============================================================================
// Domain port trait scenarios
// ============================================================================

#[given("a mock coordinator spawner that reports alive")]
fn given_spawner_alive(world: &mut QuectoWorld) {
    let spawner = Arc::new(BddMockSpawner::already_alive(12345));
    world.coord_spawner = Some(spawner);
}

#[given("a mock coordinator spawner that reports not alive")]
fn given_spawner_not_alive(world: &mut QuectoWorld) {
    let spawner = Arc::new(BddMockSpawner::needs_spawn(99999));
    world.coord_spawner = Some(spawner);
}

#[given("a mock coordinator spawner that fails to spawn")]
fn given_spawner_fails(world: &mut QuectoWorld) {
    let spawner = Arc::new(BddMockSpawner::failing());
    world.coord_spawner = Some(spawner);
}

#[when("the spawner is asked to ensure the coordinator is alive")]
fn when_ensure_alive(world: &mut QuectoWorld) {
    let spawner = world.coord_spawner.as_ref().expect("spawner set");
    world.coord_spawn_result = Some(spawner.ensure_alive());
}

#[then("the spawner should return the existing PID")]
fn then_existing_pid(world: &mut QuectoWorld) {
    let result = world.coord_spawn_result.as_ref().expect("spawn result set");
    let sr = result.as_ref().expect("should be ok");
    assert!(!sr.spawned, "should not have spawned");
    assert!(sr.pid > 0, "PID should be non-zero");
}

#[then("the spawner should return a new PID")]
fn then_new_pid(world: &mut QuectoWorld) {
    let result = world.coord_spawn_result.as_ref().expect("spawn result set");
    let sr = result.as_ref().expect("should be ok");
    assert!(sr.spawned, "should have spawned");
    assert!(sr.pid > 0, "PID should be non-zero");
}

#[then("the spawner should have launched exactly 1 process")]
fn then_one_launch(world: &mut QuectoWorld) {
    let spawner = world.coord_spawner.as_ref().expect("spawner set");
    let launches = *spawner.spawn_launches.lock().unwrap();
    assert_eq!(launches, 1, "should have launched 1 process");
}

#[then(regex = r#"^the spawner should return an error containing "([^"]+)"$"#)]
fn then_spawn_error(world: &mut QuectoWorld, expected: String) {
    let result = world.coord_spawn_result.as_ref().expect("spawn result set");
    let err = result.as_ref().expect_err("should be error");
    assert!(
        err.contains(&expected),
        "error should contain '{expected}', got: {err}"
    );
}

// ============================================================================
// Delegation tool with auto-spawn scenarios
// ============================================================================

#[given("a coordinator delegation tool with auto-spawn enabled")]
fn given_delegation_tool_with_spawn(world: &mut QuectoWorld) {
    let mock_ipc = Arc::new(BddDelegMockIpc::new());
    world.deleg_mock_ipc = Some(mock_ipc.clone());
    // Spawner will be set by subsequent Given steps
    world.deleg_tool = None; // placeholder, built lazily
}

#[given(regex = r#"^the mock spawner reports the coordinator is not alive$"#)]
fn given_spawner_not_alive_for_tool(world: &mut QuectoWorld) {
    let spawner = Arc::new(BddMockSpawner::needs_spawn(55555));
    world.coord_spawner = Some(spawner.clone());
    // Rebuild the delegation tool with this spawner
    let ipc = world.deleg_mock_ipc.as_ref().expect("mock ipc set").clone();
    world.deleg_tool = Some(Arc::new(
        CoordinatorDelegationTool::with_spawner_and_polling(ipc, spawner, 1, 3),
    ));
}

#[given(regex = r#"^the mock spawner reports the coordinator is alive with PID (\d+)$"#)]
fn given_spawner_alive_for_tool(world: &mut QuectoWorld, pid: u32) {
    let spawner = Arc::new(BddMockSpawner::already_alive(pid));
    world.coord_spawner = Some(spawner.clone());
    let ipc = world.deleg_mock_ipc.as_ref().expect("mock ipc set").clone();
    world.deleg_tool = Some(Arc::new(
        CoordinatorDelegationTool::with_spawner_and_polling(ipc, spawner, 1, 3),
    ));
}

#[given("the mock spawner fails to spawn")]
fn given_spawner_fails_for_tool(world: &mut QuectoWorld) {
    let spawner = Arc::new(BddMockSpawner::failing());
    world.coord_spawner = Some(spawner.clone());
    let ipc = world.deleg_mock_ipc.as_ref().expect("mock ipc set").clone();
    world.deleg_tool = Some(Arc::new(
        CoordinatorDelegationTool::with_spawner_and_polling(ipc, spawner, 1, 3),
    ));
}

#[then("the spawner should have been called to ensure alive")]
fn then_spawner_called(world: &mut QuectoWorld) {
    let spawner = world.coord_spawner.as_ref().expect("spawner set");
    let calls = *spawner.ensure_alive_calls.lock().unwrap();
    assert!(calls >= 1, "spawner should have been called at least once");
}

#[then("the spawner should not have launched any process")]
fn then_no_launches(world: &mut QuectoWorld) {
    let spawner = world.coord_spawner.as_ref().expect("spawner set");
    let launches = *spawner.spawn_launches.lock().unwrap();
    assert_eq!(launches, 0, "should not have launched any process");
}

// ============================================================================
// Spawn configuration scenarios
// ============================================================================

#[given(regex = r#"^a coordinator process spawner with heartbeat interval (\d+) seconds$"#)]
fn given_spawner_with_heartbeat_interval(world: &mut QuectoWorld, secs: u64) {
    let ipc = Arc::new(BddDelegMockIpc::new());
    let config =
        CoordinatorSpawnConfig::new(PathBuf::from("/tmp/test")).with_heartbeat_interval(secs);
    world.coord_process_spawner = Some(CoordinatorProcessSpawner::new(ipc, config));
}

#[given("a coordinator process spawner with default config")]
fn given_spawner_default(world: &mut QuectoWorld) {
    let ipc = Arc::new(BddDelegMockIpc::new());
    let config = CoordinatorSpawnConfig::new(PathBuf::from("/tmp/test"));
    world.coord_process_spawner = Some(CoordinatorProcessSpawner::new(ipc, config));
}

#[then(regex = r#"^the spawner heartbeat interval should be (\d+)$"#)]
fn then_spawner_heartbeat_interval(world: &mut QuectoWorld, expected: u64) {
    let spawner = world
        .coord_process_spawner
        .as_ref()
        .expect("process spawner set");
    assert_eq!(spawner.heartbeat_interval_secs(), expected);
}
