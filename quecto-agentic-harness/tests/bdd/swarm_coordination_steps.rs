use super::{QuectoWorld, result, result_json, run};
use cucumber::{then, when};
use serde_json::json;

#[when("a swarm member creates an acceptance task")]
fn create_task(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; print(board.task_create('acceptance', 'implement behavior', ['tests pass'], []))"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
}

#[then("a later swarm execution sees the acceptance task")]
fn durable_task(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; print(board.task(1)['title'])"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    assert_eq!(
        result_json(world)["stdout"].as_str().unwrap().trim(),
        "implement behavior"
    );
}

#[when("a swarm member claims work with an unmet dependency")]
fn dependency(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; a=board.task_create('a','first',['pass'],[]); b=board.task_create('b','second',['pass'],[a['id']]); board.claim(b['id'])"}),
    );
}

#[when("a swarm member submits its work")]
fn submit(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; a=board.task_create('a','first',['pass'],[]); c=board.claim(a['id']); board.submit(a['id'],c['token'],[{'artifact':'tests.log','revision':'abc'}])"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    run(world, json!({"op":"summary"}));
}

#[then(expr = "the swarm task status is {string}")]
fn task_status(world: &mut QuectoWorld, status: String) {
    assert_eq!(result_json(world)["tasks"][0]["status"], status);
}

#[when("the swarm coordinator attempts completion without evidence")]
fn false_completion(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board; board.complete('abc')"}),
    );
}

#[when("the swarm parent cancels the run")]
fn cancel(world: &mut QuectoWorld) {
    run(world, json!({"op":"cancel_run"}));
    assert!(!result(world).is_error, "{}", result(world).content);
}

#[then(expr = "the swarm run status is {string}")]
fn run_status(world: &mut QuectoWorld, status: String) {
    assert_eq!(result_json(world)["status"], status);
}

#[then("the swarm summary retains one task")]
fn partial_summary(world: &mut QuectoWorld) {
    assert_eq!(result_json(world)["tasks"].as_array().unwrap().len(), 1);
}

#[when("swarm members contend for an overlapping file set")]
fn overlap(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"run","code":"from swarm import board, SwarmError\na=board.task_create('a','first',['pass'],[])\nb=board.task_create('b','second',['pass'],[])\nx=board.claim(a['id']); y=board.claim(b['id'])\nboard.reserve(a['id'],x['token'],['a.rs'])\ntry:\n board.reserve(b['id'],y['token'],['free.rs','./a.rs'])\nexcept SwarmError:\n pass"}),
    );
    assert!(!result(world).is_error, "{}", result(world).content);
    run(world, json!({"op":"summary"}));
}

#[then("only the first swarm file set is owned")]
fn ownership(world: &mut QuectoWorld) {
    let summary = result_json(world);
    let files = summary["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "a.rs");
    assert_eq!(files[0]["task"], 1);
}
