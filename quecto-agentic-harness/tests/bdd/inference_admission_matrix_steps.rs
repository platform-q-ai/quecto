//! P4 integrated AC matrix (AC2/AC3/AC6) over a real in-process authority.
//! Reuses the authority scenario state; only adds the policies (reserve,
//! queue bound, pacing) and the child/background registrations it needs.

use super::inference_admission_authority_steps::{
    LIMIT, group, inspect, new_root, proposal, rt, wait_for,
};
use super::*;
use quecto::application::ports::AttemptPermit;
use quecto::domain::inference_admission::{Feedback, WorkloadClass};
use quecto::infrastructure::admission::{AuthorityConnection, AuthorityDirectory, AuthorityServer};
use std::time::{Duration, Instant};

type Pending = tokio::task::JoinHandle<Result<Box<dyn AttemptPermit>, String>>;

#[derive(Default)]
pub struct MatrixState {
    child: Option<AuthorityConnection>,
    backgrounds: Vec<AuthorityConnection>,
    child_pending: Option<Pending>,
    second_pending: Option<Pending>,
    refusal: Option<String>,
    first_granted_at: Option<Instant>,
    second_granted_at: Option<Instant>,
}
impl std::fmt::Debug for MatrixState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<admission matrix scenario>")
    }
}

fn start_with(
    world: &mut QuectoWorld,
    mutate: impl FnOnce(&mut quecto::domain::inference_admission::GroupPolicy),
) {
    let s = &mut world.authority;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let dir = AuthorityDirectory::open(&temp.path().join("authority")).unwrap();
    let mut proposal = proposal(2, 60_000);
    mutate(proposal.policy.groups.get_mut(&group()).unwrap());
    let server = runtime
        .block_on(AuthorityServer::start(dir, proposal))
        .unwrap();
    s.runtime = Some(runtime);
    s.temp = Some(temp);
    s.server = Some(server);
}

fn acquire_now(world: &mut QuectoWorld, root: usize) -> Box<dyn AttemptPermit> {
    let s = &world.authority;
    let gate = s.roots[root].gate("acct").unwrap();
    rt(s)
        .block_on(async { tokio::time::timeout(LIMIT, gate.acquire()).await })
        .expect("bounded wait")
        .expect("granted")
}

fn spawn_acquire(world: &QuectoWorld, connection: &AuthorityConnection) -> Pending {
    let gate = connection.gate("acct").unwrap();
    rt(&world.authority).spawn(async move { gate.acquire().await.map_err(|e| e.to_string()) })
}

fn join(world: &QuectoWorld, pending: Pending) -> Result<Box<dyn AttemptPermit>, String> {
    rt(&world.authority)
        .block_on(async { tokio::time::timeout(LIMIT, pending).await })
        .expect("bounded wait")
        .expect("join")
}

#[given("the first root has registered a bound child")]
fn given_child(world: &mut QuectoWorld) {
    let s = &world.authority;
    let socket = s.server.as_ref().unwrap().directory().client_socket();
    let parent = &s.roots[0];
    let child = rt(s).block_on(async {
        let credential = parent.register_child().await.unwrap();
        let connection = AuthorityConnection::connect(&socket).await.unwrap();
        connection.bind(credential).await.unwrap();
        connection
    });
    world.admission_matrix.child = Some(child);
}

#[when("the first root holds the slot while its child and then the second root queue")]
fn when_child_then_second_queue(world: &mut QuectoWorld) {
    let permit = acquire_now(world, 0);
    world.authority.permits.push(permit);
    let child = world.admission_matrix.child.as_ref().unwrap();
    let child_pending = spawn_acquire(world, child);
    wait_for(&world.authority, "child queued", |g| g.queued == 1);
    let second = spawn_acquire(world, &world.authority.roots[1]);
    wait_for(&world.authority, "second root queued", |g| g.queued == 2);
    world.admission_matrix.child_pending = Some(child_pending);
    world.admission_matrix.second_pending = Some(second);
}

#[when("the first root completes its attempt")]
fn when_first_completes(world: &mut QuectoWorld) {
    let permit = world.authority.permits.pop().expect("held permit");
    let _guard = rt(&world.authority).enter();
    permit.finish(Feedback::Success);
}

#[then("the second root is granted before the first root's child")]
fn then_second_root_first(world: &mut QuectoWorld) {
    let pending = world.admission_matrix.second_pending.take().unwrap();
    let permit = join(world, pending).expect("second root granted");
    assert!(
        !world
            .admission_matrix
            .child_pending
            .as_ref()
            .unwrap()
            .is_finished(),
        "the child of the root that just had service must wait its root's next turn"
    );
    let status = inspect(&world.authority);
    let g = status.groups[&group()];
    assert_eq!((g.active, g.queued), (1, 1), "{refusal}: {g:?}");
}

#[then("the child is granted once the second root completes")]
fn then_child_after(world: &mut QuectoWorld) {
    when_first_completes(world);
    let pending = world.admission_matrix.child_pending.take().unwrap();
    let permit = join(world, pending).expect("child granted");
    let _guard = rt(&world.authority).enter();
    permit.finish(Feedback::Success);
    wait_for(&world.authority, "slot released", |g| {
        g.active == 0 && g.queued == 0
    });
}

#[given("a running admission authority with capacity two and an interactive reserve of one")]
fn given_reserve(world: &mut QuectoWorld) {
    start_with(world, |g| {
        g.capacity = 2;
        g.reserve = 1;
    });
}

#[when("two background roots request inference")]
fn when_backgrounds(world: &mut QuectoWorld) {
    let s = &world.authority;
    let socket = s.server.as_ref().unwrap().directory().client_socket();
    let connect = || {
        rt(s).block_on(async {
            let connection = AuthorityConnection::connect(&socket).await.unwrap();
            let credential = connection
                .register_root(WorkloadClass::Background)
                .await
                .unwrap();
            connection.bind(credential).await.unwrap();
            connection
        })
    };
    let (first, second) = (connect(), connect());
    let permit = rt(s)
        .block_on(async {
            tokio::time::timeout(LIMIT, first.gate("acct").unwrap().acquire()).await
        })
        .unwrap()
        .expect("first background granted");
    let pending = spawn_acquire(world, &second);
    world.authority.permits.push(permit);
    world.admission_matrix.backgrounds.extend([first, second]);
    world.admission_matrix.second_pending = Some(pending);
}

#[then("only one background attempt is active and the other waits")]
fn then_one_background(world: &mut QuectoWorld) {
    wait_for(&world.authority, "one active, one queued", |g| {
        g.active == 1 && g.queued == 1
    });
    assert!(
        !world
            .admission_matrix
            .second_pending
            .as_ref()
            .unwrap()
            .is_finished(),
        "background must not take the interactive reserve"
    );
}

#[when("an interactive root requests inference")]
fn when_interactive(world: &mut QuectoWorld) {
    let root = new_root(&world.authority);
    world.authority.roots.push(root);
    let permit = acquire_now(world, 0);
    world.authority.permits.push(permit);
}

#[then("the interactive root is granted immediately alongside the background attempt")]
fn then_interactive_granted(world: &mut QuectoWorld) {
    let status = inspect(&world.authority);
    assert_eq!(
        status.groups[&group()].active,
        2,
        "{:?}",
        status.groups[&group()]
    );
    assert_eq!(status.groups[&group()].queued, 1, "background still waits");
    assert!(
        !world
            .admission_matrix
            .second_pending
            .as_ref()
            .unwrap()
            .is_finished()
    );
    let pending = world.admission_matrix.second_pending.take().unwrap();
    pending.abort();
}

#[given(
    "a running admission authority with capacity one, a queue of one and a short queue deadline"
)]
fn given_bounded_queue(world: &mut QuectoWorld) {
    start_with(world, |g| {
        g.capacity = 1;
        g.queue_capacity = 1;
        // Long enough that a slow, coverage-instrumented runner still
        // observes the queue-full refusal before the queued wait expires.
        g.queue_timeout_ms = 10_000;
    });
}

#[when("a second root queues and a third root requests")]
fn when_second_and_third(world: &mut QuectoWorld) {
    let second = new_root(&world.authority);
    let third = new_root(&world.authority);
    let pending = spawn_acquire(world, &second);
    wait_for(&world.authority, "second queued", |g| g.queued == 1);
    let refusal = rt(&world.authority)
        .block_on(async {
            tokio::time::timeout(LIMIT, third.gate("acct").unwrap().acquire()).await
        })
        .expect("bounded wait")
        .err()
        .map(|e| e.to_string());
    world.authority.roots.extend([second, third]);
    world.admission_matrix.second_pending = Some(pending);
    world.admission_matrix.refusal = refusal;
}

#[then("the third root is refused as queue full without a grant")]
fn then_queue_full(world: &mut QuectoWorld) {
    let refusal = world.admission_matrix.refusal.as_deref().expect("refused");
    assert!(refusal.to_ascii_lowercase().contains("queue"), "{refusal}");
    let status = inspect(&world.authority);
    assert_eq!(
        (
            status.groups[&group()].active,
            status.groups[&group()].queued
        ),
        (1, 1)
    );
}

#[then("the second root's wait ends with an explicit deadline error and no grant")]
fn then_deadline(world: &mut QuectoWorld) {
    let pending = world.admission_matrix.second_pending.take().unwrap();
    let error = join(world, pending).expect_err("no grant after the deadline");
    assert!(error.contains("deadline"), "{error}");
    wait_for(&world.authority, "queue drained", |g| {
        g.active == 1 && g.queued == 0
    });
}

#[when("the authority stops")]
fn when_authority_stops(world: &mut QuectoWorld) {
    let server = world.authority.server.take().unwrap();
    rt(&world.authority).block_on(server.shutdown());
}

#[then("a root's next attempt fails explicitly within the bounded wait")]
fn then_fails_closed(world: &mut QuectoWorld) {
    let s = &world.authority;
    let gate = s.roots[0].gate("acct").unwrap();
    let outcome = rt(s).block_on(async { tokio::time::timeout(LIMIT, gate.acquire()).await });
    let error = outcome
        .expect("bounded, never an unbounded fallback")
        .expect_err("no grant without an authority");
    let text = error.to_string();
    assert!(
        text.contains("admission authority connection closed"),
        "explicit outage, not a fallback: {text}"
    );
}

#[given("a running admission authority with capacity two and a start interval of 200 milliseconds")]
fn given_paced(world: &mut QuectoWorld) {
    start_with(world, |g| {
        g.capacity = 2;
        g.min_interval_ms = 200;
    });
}

#[when("both roots request inference back to back")]
fn when_back_to_back(world: &mut QuectoWorld) {
    // Measured from before the first request: the second start is at least
    // one interval after the first start, which is itself after this instant.
    world.admission_matrix.first_granted_at = Some(Instant::now());
    let first = acquire_now(world, 0);
    let second = acquire_now(world, 1);
    world.admission_matrix.second_granted_at = Some(Instant::now());
    let _guard = rt(&world.authority).enter();
    first.finish(Feedback::Success);
    second.finish(Feedback::Success);
}

#[then("the second grant starts no earlier than the pacing interval after the first")]
fn then_paced(world: &mut QuectoWorld) {
    let m = &world.admission_matrix;
    let gap = m.second_granted_at.unwrap() - m.first_granted_at.unwrap();
    assert!(
        gap >= Duration::from_millis(190),
        "second grant after {gap:?}"
    );
    wait_for(&world.authority, "both released", |g| g.active == 0);
}
