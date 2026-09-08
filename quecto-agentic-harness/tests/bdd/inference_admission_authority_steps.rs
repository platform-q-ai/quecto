//! P3 authority scenarios over a real in-process authority (UDS, journal,
//! singleton lock). Every Then observes the authority through its admin
//! socket or a client reply, never through fixture bookkeeping.
use super::*;
use quecto::application::ports::{AttemptPermit, Credential};
use quecto::domain::inference_admission::{Feedback, GroupId, GroupPolicy, WorkloadClass};
use quecto::infrastructure::admission::{
    AdminConnection, AuthorityConnection, AuthorityDirectory, AuthorityServer, ClientError,
};
use quecto::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;
use std::collections::BTreeMap;
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct AuthorityState {
    runtime: Option<tokio::runtime::Runtime>,
    temp: Option<tempfile::TempDir>,
    server: Option<AuthorityServer>,
    roots: Vec<AuthorityConnection>,
    credential: Option<Credential>,
    permits: Vec<Box<dyn AttemptPermit>>,
    pending: Option<tokio::task::JoinHandle<Result<Box<dyn AttemptPermit>, String>>>,
    refusal: Option<String>,
    epoch_before: u64,
    epoch_after: u64,
}
impl std::fmt::Debug for AuthorityState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<authority scenario>")
    }
}

fn group() -> GroupId {
    GroupId::new("g").unwrap()
}

fn proposal(capacity: usize, queue_timeout_ms: u64) -> AdmissionRuntimeProposal {
    AdmissionRuntimeProposal {
        policy: quecto::domain::inference_admission::AdmissionConfig {
            groups: BTreeMap::from([(
                group(),
                GroupPolicy {
                    capacity,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 8,
                    queue_timeout_ms,
                    attempt_timeout_ms: 60_000,
                    fallback_base_ms: 100,
                    max_cooldown_ms: 10_000,
                },
            )]),
            aliases: BTreeMap::from([("acct".into(), group())]),
            max_scopes: 16,
            terminal_capacity: 32,
        },
        bindings: BTreeMap::from([("openai".into(), "acct".into())]),
    }
}

fn state(world: &mut QuectoWorld) -> &mut AuthorityState {
    &mut world.authority
}

fn rt(state: &AuthorityState) -> &tokio::runtime::Runtime {
    state.runtime.as_ref().expect("authority runtime")
}

fn start(world: &mut QuectoWorld, capacity: usize, queue_timeout_ms: u64) {
    let s = state(world);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let dir = AuthorityDirectory::open(&temp.path().join("authority")).unwrap();
    let server = runtime
        .block_on(AuthorityServer::start(
            dir,
            proposal(capacity, queue_timeout_ms),
        ))
        .unwrap();
    s.runtime = Some(runtime);
    s.temp = Some(temp);
    s.server = Some(server);
}

fn new_root(s: &AuthorityState) -> AuthorityConnection {
    let socket = s.server.as_ref().unwrap().directory().client_socket();
    rt(s).block_on(async {
        let connection = AuthorityConnection::connect(&socket).await.unwrap();
        let credential = connection
            .register_root(WorkloadClass::Interactive)
            .await
            .unwrap();
        connection.bind(credential).await.unwrap();
        connection
    })
}

fn inspect(s: &AuthorityState) -> quecto::application::ports::AuthorityStatus {
    let admin_socket = s.server.as_ref().unwrap().directory().admin_socket();
    rt(s).block_on(async {
        AdminConnection::connect(&admin_socket)
            .await
            .unwrap()
            .inspect()
            .await
            .unwrap()
    })
}

fn wait_for(
    s: &AuthorityState,
    description: &str,
    expected: impl Fn(&quecto::domain::inference_admission::GroupSnapshot) -> bool,
) {
    let start = std::time::Instant::now();
    loop {
        let status = inspect(s);
        if expected(&status.groups[&group()]) {
            return;
        }
        assert!(
            start.elapsed() < LIMIT,
            "authority never observed {description}: {:?}",
            status.groups[&group()]
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[given("a running admission authority with capacity one")]
fn given_capacity_one(world: &mut QuectoWorld) {
    // The queued request must outlive any scheduling delay on a busy runner.
    start(world, 1, 60_000);
}

#[given("a running admission authority with capacity two")]
fn given_capacity_two(world: &mut QuectoWorld) {
    // A short queue deadline makes the quarantine refusal explicit and bounded.
    start(world, 2, 300);
}

#[given("two independent root sessions bound to the authority")]
fn given_two_roots(world: &mut QuectoWorld) {
    let s = state(world);
    let a = new_root(s);
    let b = new_root(s);
    s.roots.extend([a, b]);
}

#[given("a root session holding an active attempt")]
fn given_holding_root(world: &mut QuectoWorld) {
    let s = state(world);
    let root = new_root(s);
    s.credential = root.credential();
    let gate = root.gate("acct").unwrap();
    let permit = rt(s)
        .block_on(async { tokio::time::timeout(LIMIT, gate.acquire()).await })
        .unwrap()
        .unwrap();
    s.permits.push(permit);
    s.roots.push(root);
}

#[when("both roots request inference through the same alias")]
fn when_both_request(world: &mut QuectoWorld) {
    let s = state(world);
    let first = s.roots[0].gate("acct").unwrap();
    let permit = rt(s)
        .block_on(async { tokio::time::timeout(LIMIT, first.acquire()).await })
        .unwrap()
        .unwrap();
    s.permits.push(permit);
    let second = s.roots[1].gate("acct").unwrap();
    let pending = rt(s).spawn(async move { second.acquire().await.map_err(|e| e.to_string()) });
    s.pending = Some(pending);
}

#[then("exactly one attempt is active and the other is queued")]
fn then_one_active_one_queued(world: &mut QuectoWorld) {
    let s = state(world);
    wait_for(s, "one active and one queued", |g| {
        g.active == 1 && g.queued == 1
    });
    assert!(
        !s.pending.as_ref().unwrap().is_finished(),
        "queued root has no grant yet"
    );
}

#[when("the active root completes its attempt")]
fn when_active_completes(world: &mut QuectoWorld) {
    let s = state(world);
    let permit = s.permits.pop().expect("active permit");
    let _guard = rt(s).enter();
    permit.finish(Feedback::Success);
}

#[then("the queued root is granted")]
fn then_queued_granted(world: &mut QuectoWorld) {
    let s = state(world);
    let pending = s.pending.take().expect("pending acquire");
    let permit = rt(s)
        .block_on(async { tokio::time::timeout(LIMIT, pending).await })
        .expect("grant within the bound")
        .unwrap()
        .expect("queued root granted");
    assert_eq!(inspect(s).groups[&group()].active, 1);
    s.permits.push(permit);
}

#[when("the holding session's connection is lost")]
fn when_connection_lost(world: &mut QuectoWorld) {
    let s = state(world);
    s.epoch_before = inspect(s).epoch;
    let root = s.roots.pop().expect("holding root");
    drop(root);
}

#[then("the authority reports one uncertain attempt")]
fn then_one_uncertain(world: &mut QuectoWorld) {
    let s = state(world);
    wait_for(s, "one uncertain attempt", |g| {
        g.uncertain == 1 && g.active == 1
    });
    let snapshot = inspect(s).groups[&group()];
    assert_eq!((snapshot.uncertain, snapshot.active), (1, 1));
}

#[then("another root's request is refused without a grant")]
fn then_refused(world: &mut QuectoWorld) {
    let s = state(world);
    let other = new_root(s);
    let gate = other.gate("acct").unwrap();
    let outcome = rt(s)
        .block_on(async { tokio::time::timeout(LIMIT, gate.acquire()).await })
        .unwrap();
    let error = outcome
        .expect_err("quarantined group never grants")
        .to_string();
    assert!(error.contains("admission"), "{error}");
    s.refusal = Some(error);
    s.roots.push(other);
}

#[when("the same capability reconnects and completes the attempt")]
fn when_reconnect_completes(world: &mut QuectoWorld) {
    let s = state(world);
    let credential = s.credential.clone().expect("holding credential");
    let socket = s.server.as_ref().unwrap().directory().client_socket();
    rt(s).block_on(async {
        let again = AuthorityConnection::connect(&socket).await.unwrap();
        again.bind(credential).await.unwrap();
        again.complete(1, Feedback::Success).await.unwrap();
    });
}

#[then("the authority reports no uncertain attempts and grants again")]
fn then_grants_again(world: &mut QuectoWorld) {
    let s = state(world);
    assert_eq!(inspect(s).groups[&group()].uncertain, 0);
    let gate = s.roots.last().unwrap().gate("acct").unwrap();
    let permit = rt(s)
        .block_on(async { tokio::time::timeout(LIMIT, gate.acquire()).await })
        .unwrap()
        .expect("grant after verified termination");
    s.permits.push(permit);
}

#[when("the operator resets the authority")]
fn when_reset(world: &mut QuectoWorld) {
    let s = state(world);
    let admin_socket = s.server.as_ref().unwrap().directory().admin_socket();
    s.epoch_after = rt(s).block_on(async {
        AdminConnection::connect(&admin_socket)
            .await
            .unwrap()
            .reset()
            .await
            .unwrap()
    });
}

#[then("the epoch advances and the old capability is unauthorized")]
fn then_epoch_advances(world: &mut QuectoWorld) {
    let s = state(world);
    assert_eq!(s.epoch_after, s.epoch_before + 1);
    assert_eq!(inspect(s).epoch, s.epoch_after);
    let credential = s.credential.clone().expect("old credential");
    let socket = s.server.as_ref().unwrap().directory().client_socket();
    let outcome = rt(s).block_on(async {
        let again = AuthorityConnection::connect(&socket).await.unwrap();
        again.bind(credential).await
    });
    assert!(
        matches!(outcome, Err(ClientError::Unauthorized)),
        "{outcome:?}"
    );
}

#[then("a fresh root session is granted in the new epoch")]
fn then_fresh_root_granted(world: &mut QuectoWorld) {
    let s = state(world);
    let fresh = new_root(s);
    assert_eq!(fresh.credential().unwrap().scope.epoch, s.epoch_after);
    let gate = fresh.gate("acct").unwrap();
    let permit = rt(s)
        .block_on(async { tokio::time::timeout(LIMIT, gate.acquire()).await })
        .unwrap()
        .expect("new epoch grants");
    s.permits.push(permit);
    s.roots.push(fresh);
}
