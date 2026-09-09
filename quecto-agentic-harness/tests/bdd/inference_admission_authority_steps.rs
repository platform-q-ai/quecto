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

pub(super) const LIMIT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct AuthorityState {
    runtime: Option<tokio::runtime::Runtime>,
    temp: Option<tempfile::TempDir>,
    server: Option<AuthorityServer>,
    roots: Vec<AuthorityConnection>,
    credential: Option<Credential>,
    pub(super) permits: Vec<Box<dyn AttemptPermit>>,
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

pub(super) fn group() -> GroupId {
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

pub(super) fn rt(state: &AuthorityState) -> &tokio::runtime::Runtime {
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

pub(super) fn new_root(s: &AuthorityState) -> AuthorityConnection {
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

// ---- Operations scenarios ---------------------------------------------------

use quecto::infrastructure::admission::{FileJournal, ServerError};
use quecto::infrastructure::admission::{
    Negotiation, ProcessAdmission, negotiate, read_admission_context, write_admission_context,
};
use quecto::infrastructure::config::Config;
use quecto::interface::cli::{CliContext, run_with_output};
use std::os::unix::fs::PermissionsExt;

#[derive(Default)]
pub struct OperationsState {
    binding: Option<ProcessAdmission>,
    receipt_ms: u64,
    parent: Option<AuthorityConnection>,
    child: Option<Credential>,
    sidecar: Option<std::path::PathBuf>,
    child_binding: Option<ProcessAdmission>,
    config_path: Option<std::path::PathBuf>,
    status_json: Option<serde_json::Value>,
    reset_epoch: u64,
    corrupt_error: Option<String>,
    missing_error: Option<String>,
    root_refusal: Option<ClientError>,
    stranger: Option<AuthorityConnection>,
}
impl std::fmt::Debug for OperationsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<authority operations scenario>")
    }
}

fn ops(world: &mut QuectoWorld) -> (&mut AuthorityState, &mut OperationsState) {
    (&mut world.authority, &mut world.authority_ops)
}

fn admin_status(s: &AuthorityState) -> quecto::application::ports::AuthorityStatus {
    inspect(s)
}

#[when("an agent process negotiates a root binding")]
fn when_negotiate_root(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let directory = s.server.as_ref().unwrap().directory().path().to_path_buf();
    let unreachable = negotiate(Negotiation::Root {
        directory: directory.join("nowhere"),
    })
    .expect_err("an unreachable authority is an explicit error");
    assert!(unreachable.contains("unreachable"), "{unreachable}");
    let binding = negotiate(Negotiation::Root { directory }).unwrap();
    assert_eq!(binding.proposal().bindings["openai"], "acct");
    assert!(binding.proposal().policy.aliases.contains_key("acct"));
    o.binding = Some(binding);
}

#[when("the process reports throttle feedback on an admitted attempt")]
fn when_feedback(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let binding = o.binding.as_ref().unwrap();
    let gate = binding.connection().gate("acct").unwrap();
    let mut permit = rt(s)
        .block_on(async { tokio::time::timeout(LIMIT, gate.acquire()).await })
        .unwrap()
        .unwrap();
    let (receipt_ms, wall) = permit.receipt_clock();
    assert!(wall > std::time::UNIX_EPOCH);
    assert_eq!(permit.maximum_cooldown_ms(), 10_000);
    permit
        .feedback(quecto::domain::inference_admission::ThrottleFeedback::Until(receipt_ms + 2_000));
    permit.feedback(quecto::domain::inference_admission::ThrottleFeedback::NoHint { jitter: 0 });
    o.receipt_ms = receipt_ms;
    let _guard = rt(s).enter();
    permit.finish(Feedback::Failure);
}

#[then("the authority records the cooldown and one live scope")]
fn then_cooldown_recorded(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let floor = o.receipt_ms + 2_000;
    wait_for(s, "cooldown from feedback", |g| g.cooldown_until >= floor);
    let status = admin_status(s);
    assert!(status.groups[&group()].cooldown_until >= floor);
    assert_eq!(status.live_scopes, 1);
}

#[when("the process shuts down its binding")]
fn when_shutdown_binding(world: &mut QuectoWorld) {
    let (_, o) = ops(world);
    let binding = o.binding.take().expect("binding");
    let (drained, retired) = binding.shutdown(Duration::from_secs(3));
    assert!(drained);
    retired.unwrap();
}

#[then("the authority reports no live scopes and no occupancy")]
fn then_no_scopes(world: &mut QuectoWorld) {
    let (s, _) = ops(world);
    let status = admin_status(s);
    assert_eq!(status.live_scopes, 0);
    assert_eq!(status.groups[&group()].active, 0);
}

#[given("a parent process with a root binding")]
fn given_parent(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let parent = new_root(s);
    o.parent = Some(parent);
}

#[when("the parent writes a capability sidecar for a child")]
fn when_write_sidecar(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let parent = o.parent.as_ref().unwrap();
    let child = rt(s).block_on(parent.register_child()).unwrap();
    let dir = s.temp.as_ref().unwrap().path().join("sidecars");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("child.json");
    let endpoint = s.server.as_ref().unwrap().directory().client_socket();
    write_admission_context(&path, &endpoint, &child).unwrap();
    assert_eq!(
        read_admission_context(&path).unwrap().serial,
        child.scope.serial
    );
    o.child = Some(child);
    o.sidecar = Some(path);
}

#[when("a child process negotiates with that sidecar")]
fn when_child_negotiates(world: &mut QuectoWorld) {
    let (_, o) = ops(world);
    let context = o.sidecar.clone().unwrap();
    o.child_binding = Some(negotiate(Negotiation::Child { context }).unwrap());
}

#[then("the child is bound and the sidecar is consumed")]
fn then_child_bound(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let binding = o.child_binding.as_ref().unwrap();
    assert_eq!(
        binding.connection().credential().unwrap().scope,
        o.child.as_ref().unwrap().scope
    );
    assert!(!o.sidecar.as_ref().unwrap().exists(), "sidecar consumed");
    assert_eq!(admin_status(s).live_scopes, 2);
}

#[then("a forged or unreadable sidecar is refused before any attempt")]
fn then_forged_refused(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let dir = s.temp.as_ref().unwrap().path().join("sidecars");
    let endpoint = s.server.as_ref().unwrap().directory().client_socket();
    let mut bogus = o.child.clone().unwrap();
    bogus.token = "forged".into();
    let forged = dir.join("forged.json");
    write_admission_context(&forged, &endpoint, &bogus).unwrap();
    let err = negotiate(Negotiation::Child {
        context: forged.clone(),
    })
    .unwrap_err();
    assert!(err.contains("rejected"), "{err}");
    assert!(
        !forged.exists(),
        "a sidecar is single-use whatever the outcome"
    );
    let malformed = dir.join("malformed.json");
    std::fs::write(&malformed, b"{nope").unwrap();
    assert!(negotiate(Negotiation::Child { context: malformed }).is_err());
    let missing = negotiate(Negotiation::Child {
        context: dir.join("absent.json"),
    })
    .unwrap_err();
    assert!(missing.contains("unreadable"), "{missing}");
    assert_eq!(
        admin_status(s).groups[&group()].active,
        0,
        "no attempt was made"
    );
}

#[given("a configuration file that enables admission for that authority")]
fn given_config_file(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let directory = s.server.as_ref().unwrap().directory().path().to_path_buf();
    let path = s.temp.as_ref().unwrap().path().join("config.json");
    let config = format!(
        r#"{{"providers":{{"openai_compatible":{{"endpoints":[{{"prefix":"openai","api_key":"k","api_base":"http://127.0.0.1:1","allow_remote_http":true}}]}}}},
"admission":{{"directory":{dir:?},"groups":{{"g":{{"capacity":2,"reserve":0,"min_interval_ms":1,"queue_capacity":8,"queue_timeout_ms":300,"attempt_timeout_ms":60000,"fallback_base_ms":100,"max_cooldown_ms":10000}}}},"aliases":{{"acct":"g"}},"bindings":{{"openai":"acct"}}}}}}"#,
        dir = directory.to_string_lossy()
    );
    std::fs::write(&path, config).unwrap();
    let loaded = Config::load(path.to_str().unwrap()).unwrap();
    let (resolved, proposal) = loaded.admission_proposal().unwrap().unwrap();
    assert_eq!(resolved, directory);
    assert_eq!(
        proposal.policy.max_scopes, 1024,
        "omitted limits take defaults"
    );
    o.config_path = Some(path);
}

fn cli(o: &OperationsState, args: &[&str]) -> (i32, String, String) {
    let config = o.config_path.as_ref().unwrap();
    let mut argv = vec![
        "quecto".to_string(),
        "--config".into(),
        config.to_string_lossy().into_owned(),
    ];
    argv.extend(args.iter().map(|a| a.to_string()));
    let ctx = CliContext {
        base_dir: Some(config.parent().unwrap().to_path_buf()),
        config_path: Some(config.clone()),
        ..CliContext::default()
    };
    let output = run_with_output(argv, &ctx);
    (output.exit_code, output.stdout, output.stderr)
}

#[when("the operator runs the status command")]
fn when_status(world: &mut QuectoWorld) {
    let (_, o) = ops(world);
    let (code, out, err) = cli(o, &["admission-broker", "status"]);
    assert_eq!(code, 0, "{err}");
    o.status_json = Some(serde_json::from_str(out.trim()).unwrap());
}

#[then("the status JSON reports the epoch and no live scopes")]
fn then_status_json(world: &mut QuectoWorld) {
    let (_, o) = ops(world);
    let status = o.status_json.as_ref().unwrap();
    assert_eq!(status["epoch"], 1);
    assert_eq!(status["live_scopes"], 0);
    assert_eq!(status["journal_healthy"], true);
    assert_eq!(status["groups"]["g"]["queued"], 0);
}

#[when("the operator runs the reset command")]
fn when_reset_cli(world: &mut QuectoWorld) {
    let (_, o) = ops(world);
    let (code, out, err) = cli(o, &["admission-broker", "reset"]);
    assert_eq!(code, 0, "{err}");
    let reply: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    o.reset_epoch = reply["epoch"].as_u64().unwrap();
}

#[then("the reset advances the epoch and misuse of the command is refused explicitly")]
fn then_reset_and_misuse(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    assert_eq!(o.reset_epoch, 2);
    assert_eq!(admin_status(s).epoch, 2);
    let (code, _, err) = cli(o, &["admission-broker"]);
    assert_eq!(code, 2);
    assert!(err.contains("expected one of"), "{err}");
    let (code, _, err) = cli(o, &["admission-broker", "bogus"]);
    assert_eq!(code, 2);
    assert!(err.contains("unknown action"), "{err}");
    let (code, _, err) = cli(o, &["admission-broker", "run", "--nope"]);
    assert_eq!(code, 2);
    assert!(err.contains("unknown option"), "{err}");
    let config = o.config_path.clone().unwrap();
    let original = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, r#"{"providers":{"anthropic":{"api_key":"k"}}}"#).unwrap();
    let (code, _, err) = cli(o, &["admission-broker", "status"]);
    assert_eq!(code, 1);
    assert!(err.contains("no `admission` section"), "{err}");
    let relative = original.replacen(
        &format!(
            "\"directory\":{:?}",
            s.server
                .as_ref()
                .unwrap()
                .directory()
                .path()
                .to_string_lossy()
        ),
        "\"directory\":\"relative/dir\"",
        1,
    );
    std::fs::write(&config, relative).unwrap();
    let (code, _, err) = cli(o, &["admission-broker", "status"]);
    assert_eq!(code, 1);
    assert!(err.contains("absolute"), "{err}");
    std::fs::write(&config, original).unwrap();
}

#[when("the authority stops and its ledger is corrupted")]
fn when_stop_and_corrupt(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let server = s.server.take().unwrap();
    let dir = server.directory().clone();
    rt(s).block_on(server.shutdown());
    std::fs::write(dir.journal_path(), b"{broken").unwrap();
    o.corrupt_error = Some(format!("{:?}", FileJournal::load(&dir).unwrap_err()));
    std::fs::write(
        dir.journal_path(),
        br#"{"format":9,"epoch":1,"outstanding":[],"groups":{}}"#,
    )
    .unwrap();
    let unsupported = format!("{:?}", FileJournal::load(&dir).unwrap_err());
    assert!(unsupported.contains("format"), "{unsupported}");
    // A valid ledger restarts normally; the shutdown above persisted one.
    std::fs::write(
        dir.journal_path(),
        br#"{"format":1,"epoch":1,"outstanding":[],"groups":{}}"#,
    )
    .unwrap();
    s.server = Some(
        rt(s)
            .block_on(AuthorityServer::start(dir, proposal(2, 300)))
            .unwrap(),
    );
}

#[then("the corrupt ledger is refused as unreadable")]
fn then_corrupt_refused(world: &mut QuectoWorld) {
    let (_, o) = ops(world);
    let error = o.corrupt_error.as_ref().unwrap();
    assert!(error.contains("corrupt"), "{error}");
}

#[when("the ledger is removed after prior operation")]
fn when_ledger_removed(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let running = s.server.take().expect("restarted authority");
    rt(s).block_on(running.shutdown());
    let dir = AuthorityDirectory::open(&s.temp.as_ref().unwrap().path().join("authority")).unwrap();
    std::fs::remove_file(dir.journal_path()).unwrap();
    o.missing_error = Some(
        match rt(s).block_on(AuthorityServer::start(dir, proposal(2, 300))) {
            Err(ServerError::Journal(message)) => message,
            Err(other) => panic!("unexpected {other}"),
            Ok(_) => panic!("empty ledger accepted silently"),
        },
    );
}

#[then("a restart is refused until the operator accepts an empty ledger")]
fn then_restart_refused(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let message = o.missing_error.as_ref().unwrap();
    assert!(message.contains("missing"), "{message}");
    let dir = AuthorityDirectory::open(&s.temp.as_ref().unwrap().path().join("authority")).unwrap();
    let accepted = rt(s)
        .block_on(AuthorityServer::start_accepting_missing_ledger(
            dir.clone(),
            proposal(2, 300),
        ))
        .unwrap();
    assert!(dir.journal_path().exists(), "fresh ledger persisted");
    let busy = rt(s).block_on(AuthorityServer::start(dir, proposal(2, 300)));
    assert!(matches!(busy, Err(ServerError::Busy(_))), "singleton");
    if let Err(error) = busy {
        assert!(error.to_string().contains("another"));
    }
    s.server = Some(accepted);
}

#[when("a connection without the owner token tries to register a root")]
fn when_root_without_token(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let dir = s.server.as_ref().unwrap().directory().clone();
    let stranger = rt(s)
        .block_on(AuthorityConnection::connect(&dir.client_socket()))
        .unwrap();
    o.root_refusal = rt(s)
        .block_on(stranger.register_root_with_token(WorkloadClass::Interactive, "forged"))
        .err();
    o.stranger = Some(stranger);
}

#[then("root registration is refused and the owner token succeeds")]
fn then_root_refused(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    assert_eq!(o.root_refusal, Some(ClientError::Unauthorized));
    let dir = s.server.as_ref().unwrap().directory().clone();
    let mode = std::fs::metadata(dir.root_token_path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let stranger = o.stranger.as_ref().unwrap();
    let credential = rt(s)
        .block_on(stranger.register_root(WorkloadClass::Interactive))
        .expect("owner token read from the authority directory");
    rt(s).block_on(stranger.bind(credential)).unwrap();
    assert!(format!("{stranger:?}").contains("bound: true"));
}

#[then("a retired child capability no longer binds and unknown aliases are refused")]
fn then_retired_child(world: &mut QuectoWorld) {
    let (s, o) = ops(world);
    let parent = o.stranger.as_ref().unwrap();
    let child = rt(s).block_on(parent.register_child()).unwrap();
    rt(s).block_on(parent.retire_child(child.scope)).unwrap();
    let dir = s.server.as_ref().unwrap().directory().clone();
    let other = rt(s)
        .block_on(AuthorityConnection::connect(&dir.client_socket()))
        .unwrap();
    assert_eq!(
        rt(s).block_on(other.bind(child)),
        Err(ClientError::Unauthorized)
    );
    let unpublished = parent.gate("nope").err().unwrap().to_string();
    assert!(unpublished.contains("not published"), "{unpublished}");
    rt(s).block_on(parent.retire()).unwrap();
    assert!(parent.credential().is_none());
}

#[when("the holder cancels the granted attempt without a transport")]
fn when_cancel_granted(world: &mut QuectoWorld) {
    let (s, _) = ops(world);
    let permit = s.permits.pop().expect("held permit");
    std::mem::forget(permit);
    s.roots.last().unwrap().cancel_detached(1);
}

#[then("the slot is released as a finished attempt with no uncertainty")]
fn then_slot_released(world: &mut QuectoWorld) {
    let (s, _) = ops(world);
    wait_for(s, "released slot", |g| g.active == 0);
    let status = admin_status(s);
    assert_eq!(status.groups[&group()].uncertain, 0);
    assert_eq!(
        rt(s).block_on(s.roots.last().unwrap().status(1)).unwrap(),
        quecto::domain::inference_admission::RequestState::Terminal(
            quecto::domain::inference_admission::TerminalOutcome::Finished
        )
    );
}
