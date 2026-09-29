//! #2281: the board steps driven through structured swarm ops, and the
//! `op`/`create`/`claim` helpers the other swarm coordination steps share.

use super::{QuectoWorld, result, result_json, run};
use cucumber::{then, when};
use serde_json::json;

/// #2279/#2281: a structured op through the coordinator's tool, which must
/// answer; its answer.
pub(super) fn op(world: &mut QuectoWorld, request: serde_json::Value) -> serde_json::Value {
    run(world, request.clone());
    assert!(
        !result(world).is_error,
        "{request}: {}",
        result(world).content
    );
    result_json(world)
}

/// A task `title` with `acceptance` (and `dependencies`) the coordinator
/// creates through `task_create`; its id.
pub(super) fn create(
    world: &mut QuectoWorld,
    title: &str,
    dependencies: serde_json::Value,
) -> serde_json::Value {
    let task = op(
        world,
        json!({"op":"task_create","request":title,"title":title,"acceptance":["pass"],
            "dependencies":dependencies}),
    );
    task["id"].clone()
}

/// The coordinator's claim of task `id` through `claim`; its token.
pub(super) fn claim(world: &mut QuectoWorld, id: &serde_json::Value) -> serde_json::Value {
    op(world, json!({"op":"claim","task_id":id}))["token"].clone()
}

#[when("a swarm member creates an acceptance task")]
fn create_task(world: &mut QuectoWorld) {
    let task = op(
        world,
        json!({"op":"task_create","request":"acceptance","title":"implement behavior",
            "acceptance":["tests pass"],"dependencies":[]}),
    );
    assert_eq!(task["id"], 1, "{task}");
}

#[then("a later swarm execution sees the acceptance task")]
fn durable_task(world: &mut QuectoWorld) {
    let task = op(world, json!({"op":"task","task_id":1}));
    assert_eq!(task["title"], "implement behavior", "{task}");
}

#[when("a swarm member claims work with an unmet dependency")]
fn dependency(world: &mut QuectoWorld) {
    let first = create(world, "first", json!([]));
    let second = create(world, "second", json!([first]));
    run(world, json!({"op":"claim","task_id":second}));
}

#[when("a swarm member submits its work")]
fn submit(world: &mut QuectoWorld) {
    let id = create(world, "first", json!([]));
    let token = claim(world, &id);
    op(
        world,
        json!({"op":"submit","task_id":id,"token":token,
            "evidence":[{"artifact":"tests.log","revision":"abc"}]}),
    );
    run(world, json!({"op":"summary"}));
}

#[then(expr = "the swarm task status is {string}")]
fn task_status(world: &mut QuectoWorld, status: String) {
    assert_eq!(result_json(world)["tasks"][0]["status"], status);
}

#[when("the swarm coordinator attempts completion without evidence")]
fn false_completion(world: &mut QuectoWorld) {
    run(world, json!({"op":"complete","revision":"abc"}));
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
    let (a, b) = (
        create(world, "first", json!([])),
        create(world, "second", json!([])),
    );
    let (x, y) = (claim(world, &a), claim(world, &b));
    op(
        world,
        json!({"op":"reserve","task_id":a,"token":x,"paths":["a.rs"]}),
    );
    // The overlapping set is refused whole (the program caught the refusal).
    run(
        world,
        json!({"op":"reserve","task_id":b,"token":y,"paths":["free.rs","./a.rs"]}),
    );
    assert!(result(world).is_error, "{}", result(world).content);
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

#[when("the coordinator completes dependent tasks at different revisions")]
fn dependent_revisions(world: &mut QuectoWorld) {
    fn verified(
        world: &mut QuectoWorld,
        title: &str,
        dependencies: serde_json::Value,
        revision: &str,
    ) -> serde_json::Value {
        let id = create(world, title, dependencies);
        let token = claim(world, &id);
        let evidence = json!([{"artifact":format!("{title}.log"),"revision":revision}]);
        op(
            world,
            json!({"op":"submit","task_id":id,"token":token,"evidence":evidence}),
        );
        op(
            world,
            json!({"op":"verify_task","task_id":id,"token":token,"revision":revision}),
        );
        id
    }
    let first = verified(world, "a", json!([]), "R1");
    verified(world, "b", json!([first]), "R2");
    op(
        world,
        json!({"op":"evidence","criterion":"tests","artifact":"final.log","revision":"R2",
            "kind":"command","passed":true}),
    );
    run(world, json!({"op":"complete","revision":"R2"}));
    assert!(result(world).is_error);
    assert!(result(world).content.contains("stale revision"));
}

#[when("revalidates earlier work with fresh final revision evidence")]
fn revalidate_revision(world: &mut QuectoWorld) {
    op(
        world,
        json!({"op":"revalidate_task","task_id":1,"revision":"R2",
            "evidence":[{"artifact":"a-rerun.log","revision":"R2"}]}),
    );
    op(world, json!({"op":"complete","revision":"R2"}));
    run(world, json!({"op":"summary"}));
}

#[when("a swarm member supplies acceptance as a string")]
fn acceptance_type(world: &mut QuectoWorld) {
    run(
        world,
        json!({"op":"task_create","request":"invalid","title":"work","acceptance":"tests pass"}),
    );
}

#[when("a swarm task awaits master approval")]
fn await_approval(world: &mut QuectoWorld) {
    let task = op(
        world,
        json!({"op":"task_create","request":"approval","title":"wishlist",
            "acceptance":["approved schema"]}),
    );
    let token = claim(world, &task["id"]);
    op(
        world,
        json!({"op":"block","task_id":task["id"],"token":token,"reason":"awaiting master approval"}),
    );
    run(world, json!({"op":"summary"}));
}

#[when("the approved swarm task is completed")]
fn apply_approval(world: &mut QuectoWorld) {
    let task = op(world, json!({"op":"task","task_id":1}));
    op(
        world,
        json!({"op":"submit","task_id":1,"token":task["token"],
            "evidence":[{"artifact":"approved-tests.log","revision":"approved-revision"}]}),
    );
    run(world, json!({"op":"summary"}));
}
