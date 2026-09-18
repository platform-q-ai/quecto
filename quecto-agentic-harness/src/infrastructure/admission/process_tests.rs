//! Process binding over a real in-process authority: root install, gates,
//! child negotiation with sidecar consumption, failure paths and shutdown.
use super::*;
use crate::domain::inference_admission::{AdmissionConfig, Feedback, GroupId, GroupPolicy};
use crate::infrastructure::admission::{AdminConnection, AuthorityDirectory, AuthorityServer};
use std::collections::BTreeMap;
use std::time::Duration;

fn proposal() -> AdmissionRuntimeProposal {
    let g = GroupId::new("g").unwrap();
    AdmissionRuntimeProposal {
        policy: AdmissionConfig {
            groups: BTreeMap::from([(
                g.clone(),
                GroupPolicy {
                    capacity: 2,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 8,
                    queue_timeout_ms: 5_000,
                    attempt_timeout_ms: 60_000,
                    fallback_base_ms: 100,
                    max_cooldown_ms: 10_000,
                },
            )]),
            aliases: BTreeMap::from([("acct".into(), g)]),
            max_scopes: 16,
            terminal_capacity: 32,
        },
        bindings: BTreeMap::from([("openai".into(), "acct".into())]),
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn context_round_trip_and_rejections() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ctx.json");
    let credential = Credential {
        scope: crate::domain::inference_admission::ScopeId {
            epoch: 3,
            serial: 9,
        },
        token: "tok".into(),
    };
    write_admission_context(
        &path,
        Path::new("/tmp/a/client/admission.sock"),
        &credential,
    )
    .unwrap();
    let context = read_admission_context(&path).unwrap();
    assert_eq!(
        (context.epoch, context.serial, context.token.as_str()),
        (3, 9, "tok")
    );
    assert_eq!(
        context.endpoint,
        PathBuf::from("/tmp/a/client/admission.sock")
    );
    assert!(
        write_admission_context(&path, Path::new("/x"), &credential).is_err(),
        "never overwrite an existing sidecar"
    );
    assert!(read_admission_context(&temp.path().join("missing.json")).is_err());
    std::fs::write(&path, b"{not json").unwrap();
    assert!(read_admission_context(&path).is_err());
    std::fs::write(
        &path,
        br#"{"format":9,"endpoint":"/x","epoch":1,"serial":1,"token":"t"}"#,
    )
    .unwrap();
    let err = read_admission_context(&path).unwrap_err();
    assert!(err.contains("format"), "{err}");
}

#[test]
fn root_install_binds_gates_and_shutdown_retires_the_scope() {
    let rt = runtime();
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("authority");
    let server = rt
        .block_on(AuthorityServer::start(
            AuthorityDirectory::open(&directory).unwrap(),
            proposal(),
        ))
        .unwrap();
    // Unreachable authority is an explicit error, never a fallback, and a
    // client never manufactures authority directories as a side effect.
    let nowhere = temp.path().join("nowhere");
    let missing = negotiate(Negotiation::Root {
        directory: nowhere.clone(),
    });
    assert!(missing.unwrap_err().contains("unreachable"));
    assert!(!nowhere.exists(), "client did not create the directory");
    // A directory that cannot be opened as an authority is named.
    let file = temp.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    let err = negotiate(Negotiation::Root { directory: file }).unwrap_err();
    assert!(err.contains("not a directory"), "{err}");
    // Without the owner token a root cannot register.
    let token = server.directory().root_token_path();
    let hidden = temp.path().join("hidden");
    std::fs::rename(&token, &hidden).unwrap();
    let err = negotiate(Negotiation::Root {
        directory: directory.clone(),
    })
    .unwrap_err();
    assert!(err.contains("root registration"), "{err}");
    std::fs::rename(&hidden, &token).unwrap();
    // Negotiation builds a binding without touching process-wide state, so
    // this test cannot leak an installed authority into sibling tests.
    let binding = negotiate(Negotiation::Root {
        directory: directory.clone(),
    })
    .unwrap();
    assert!(current().is_none());
    // Once-only install semantics against a private slot: a second, different
    // negotiation returns the first binding unchanged (restart-only policy).
    let slot: OnceLock<Arc<ProcessAdmission>> = OnceLock::new();
    let first = install_in(
        &slot,
        Negotiation::Root {
            directory: directory.clone(),
        },
    )
    .unwrap();
    let again = install_in(
        &slot,
        Negotiation::Child {
            context: temp.path().join("ignored.json"),
        },
    )
    .unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    let (drained, retired) = first.shutdown(Duration::from_secs(3));
    assert!(drained && retired.is_ok());
    assert_eq!(binding.client_dir(), server.directory().client_dir());
    assert_eq!(binding.endpoint(), server.directory().client_socket());
    assert_eq!(binding.proposal().bindings["openai"], "acct");
    assert!(binding.runtime_context().binding("openai").is_ok());
    assert!(format!("{binding:?}").contains("client_dir"));
    let gate = binding.runtime_context().binding("openai").unwrap().gate;
    // Transition hook (P4 slice 2): every transition hands the hook a view.
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    binding.on_transition(Arc::new(move |activity| {
        sink.lock().unwrap().push(activity.revision);
    }));
    let permit = rt.block_on(gate.acquire()).unwrap();
    let observed = binding.observation().snapshot();
    assert_eq!(
        observed.admitted, 1,
        "the runtime gate is observed: {observed:?}"
    );
    assert_eq!(observed.attempts.values().next().unwrap().alias, "acct");
    permit.finish(Feedback::Success);
    assert_eq!(binding.observation().snapshot().completed, 1);
    let revisions = seen.lock().unwrap().clone();
    assert_eq!(revisions, vec![1, 2, 3], "queued, granted, completed");
    let admin = rt
        .block_on(AdminConnection::connect(&server.directory().admin_socket()))
        .unwrap();
    let before = rt.block_on(admin.inspect()).unwrap();
    assert_eq!(before.live_scopes, 1);
    let (drained, retired) = binding.shutdown(Duration::from_secs(3));
    assert!(drained);
    retired.unwrap();
    let after = rt.block_on(admin.inspect()).unwrap();
    assert_eq!(after.live_scopes, 0, "shutdown retired the root scope");
    assert_eq!(after.groups[&GroupId::new("g").unwrap()].active, 0);
    // The process-wide wrapper is a no-op when nothing is installed.
    shutdown(Duration::from_millis(10));
    rt.block_on(server.shutdown());
}

#[test]
fn child_negotiation_consumes_its_sidecar_and_rejects_forgeries() {
    let rt = runtime();
    let temp = tempfile::tempdir().unwrap();
    let dir = AuthorityDirectory::open(&temp.path().join("authority")).unwrap();
    let server = rt
        .block_on(AuthorityServer::start(dir.clone(), proposal()))
        .unwrap();
    let (child, parent) = rt.block_on(async {
        let parent = super::super::client::AuthorityConnection::connect(&dir.client_socket())
            .await
            .unwrap();
        let credential = parent
            .register_root(crate::domain::inference_admission::WorkloadClass::Interactive)
            .await
            .unwrap();
        parent.bind(credential).await.unwrap();
        (parent.register_child().await.unwrap(), parent)
    });
    let context = temp.path().join("child.json");
    write_admission_context(&context, &dir.client_socket(), &child).unwrap();
    let (connection, client_dir) = rt
        .block_on(connect(&Negotiation::Child {
            context: context.clone(),
        }))
        .unwrap();
    assert!(
        !context.exists(),
        "sidecar consumed after a successful bind"
    );
    assert_eq!(client_dir, dir.client_dir());
    assert_eq!(connection.credential().unwrap().scope, child.scope);
    let forged = temp.path().join("forged.json");
    let mut bogus = child.clone();
    bogus.token = "forged".into();
    write_admission_context(&forged, &dir.client_socket(), &bogus).unwrap();
    let err = rt
        .block_on(connect(&Negotiation::Child {
            context: forged.clone(),
        }))
        .unwrap_err();
    assert!(err.contains("rejected"), "{err}");
    assert!(
        !forged.exists(),
        "a sidecar is single-use whatever the outcome"
    );
    let unreachable = temp.path().join("unreachable.json");
    write_admission_context(
        &unreachable,
        Path::new("/nonexistent/admission.sock"),
        &child,
    )
    .unwrap();
    let err = rt
        .block_on(connect(&Negotiation::Child {
            context: unreachable.clone(),
        }))
        .unwrap_err();
    assert!(err.contains("unreachable"), "{err}");
    assert!(!unreachable.exists(), "a valid token never lingers on disk");
    drop(parent);
    rt.block_on(server.shutdown());
}

/// M5c (#2024 S3): the binding kind is decided by how the process joined —
/// a root never inherits (its own reload is validated), a child always does.
#[test]
fn a_root_never_inherits_and_a_child_always_does() {
    let rt = runtime();
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("authority");
    let server = rt
        .block_on(AuthorityServer::start(
            AuthorityDirectory::open(&directory).unwrap(),
            proposal(),
        ))
        .unwrap();
    let root = negotiate(Negotiation::Root {
        directory: directory.clone(),
    })
    .unwrap();
    assert_eq!(root.kind(), BindingKind::Root);
    assert!(!root.inherits_authority());
    assert_eq!(root.directory(), directory);
    assert!(root.connected());
    assert_eq!(root.epoch(), 1);

    let child_credential = rt.block_on(root.connection().register_child()).unwrap();
    let context = temp.path().join("child.json");
    write_admission_context(&context, &root.endpoint(), &child_credential).unwrap();
    let child = negotiate(Negotiation::Child { context }).unwrap();
    assert_eq!(child.kind(), BindingKind::Child);
    assert!(child.inherits_authority());
    assert_eq!(child.directory(), directory, "derived from the client dir");
    assert!(child.connected());
    rt.block_on(server.shutdown());
}

/// A parent registers descendants through the link, so a spawn after a
/// broker restart or `reset` re-registers the root first instead of failing
/// on the revoked connection (review low, #2024 S3).
#[test]
fn a_root_registers_children_through_the_reconnecting_link() {
    let rt = runtime();
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("authority");
    let server = rt
        .block_on(AuthorityServer::start(
            AuthorityDirectory::open(&directory).unwrap(),
            proposal(),
        ))
        .unwrap();
    let root = negotiate(Negotiation::Root {
        directory: directory.clone(),
    })
    .unwrap();
    let before = rt.block_on(root.register_child()).unwrap();
    assert_eq!(before.scope.epoch, 1);
    let admin = rt
        .block_on(AdminConnection::connect(&server.directory().admin_socket()))
        .unwrap();
    assert_eq!(rt.block_on(admin.reset()).unwrap(), 2);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while root.connected() {
        assert!(std::time::Instant::now() < deadline, "revocation observed");
        std::thread::sleep(Duration::from_millis(10));
    }
    let after = rt
        .block_on(root.register_child())
        .expect("re-registered root mints a child in the new epoch");
    assert_eq!(after.scope.epoch, 2);
    assert!(root.connected());
    rt.block_on(server.shutdown());
}
