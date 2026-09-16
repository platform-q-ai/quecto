//! Executable, representation-neutral RED/compatibility contracts for issue
//! #2001 and its technically approved bounded public protocol.
//!
//! These tests deliberately do not name a catalogue file, catalogue schema,
//! rebuild command, generation, or latency threshold. Authoritative transcript
//! fixtures and normal CLI/UDS list/resume boundaries make recovery observable;
//! a derived implementation may rebuild its index or bypass it. Persisted home
//! metadata is likewise inspected semantically rather than by field name.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use quecto::application::sessions::dto::SessionListQuery;
use quecto::application::sessions::ports::SessionStore;
use quecto::domain::message::Message;
use quecto::domain::session::Session;
use quecto::domain::session_identity::SessionIdentity;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::interface::cli::{CliContext, run_with_output};

fn identity(key: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(key)
}

fn config_for(workspace: &std::path::Path, api_base: &str) -> serde_json::Value {
    serde_json::json!({
        "providers": {
            "openai": {
                "api_key": "sk-test-not-a-secret",
                "api_base": api_base
            }
        },
        "agents": {
            "defaults": {
                "model": "openai-api/gpt-4o-mini",
                "workspace": workspace
            }
        }
    })
}

fn value_contains_exact_string(value: &serde_json::Value, expected: &str) -> bool {
    match value {
        serde_json::Value::String(actual) => actual == expected,
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| value_contains_exact_string(value, expected)),
        serde_json::Value::Object(fields) => fields
            .values()
            .any(|value| value_contains_exact_string(value, expected)),
        _ => false,
    }
}

/// Representation-neutral persisted-home predicate: it fixes neither the
/// metadata object name nor its exact version key/value. It requires one
/// candidate object to have an explicitly version-labelled scalar directly
/// on that object and the canonical path within the same object's subtree.
/// Unrelated transcript ordinals, timestamps, counts, turns, and a version
/// property in a different object therefore cannot satisfy versioning.
fn contains_versioned_home_subtree(value: &serde_json::Value, canonical_home: &str) -> bool {
    match value {
        serde_json::Value::Object(fields) => {
            (fields.iter().any(|(key, value)| {
                key.to_ascii_lowercase().contains("version")
                    && matches!(
                        value,
                        serde_json::Value::Number(_) | serde_json::Value::String(_)
                    )
            }) && fields
                .values()
                .any(|value| value_contains_exact_string(value, canonical_home)))
                || fields
                    .values()
                    .any(|value| contains_versioned_home_subtree(value, canonical_home))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| contains_versioned_home_subtree(value, canonical_home)),
        _ => false,
    }
}

fn authoritative_json_values(base: &Path) -> Vec<serde_json::Value> {
    std::fs::read_dir(base.join("sessions"))
        .expect("successful writer creates sessions directory")
        .filter_map(|entry| {
            let path = entry.expect("session directory entry").path();
            (path.extension().and_then(|extension| extension.to_str()) == Some("json"))
                .then_some(path)
        })
        .flat_map(|path| {
            let text = std::fs::read_to_string(path).expect("authoritative UTF-8 record");
            serde_json::from_str::<serde_json::Value>(&text)
                .map(|value| vec![value])
                .unwrap_or_else(|_| {
                    text.lines()
                        .filter(|line| !line.trim().is_empty())
                        .map(|line| serde_json::from_str(line).expect("authoritative JSONL value"))
                        .collect()
                })
        })
        .collect()
}

#[test]
fn real_writer_persists_versioned_optional_home_without_rewriting_opaque_key() {
    let runtime = tokio::runtime::Runtime::new().expect("test runtime");
    let server = runtime.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "choices": [{
                        "message": {"role": "assistant", "content": "saved"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                })),
            )
            .mount(&server)
            .await;
        server
    });
    let base = tempfile::tempdir().unwrap();
    let workspace = base.path().join("canonical-home");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        base.path().join("config.json"),
        serde_json::to_vec(&config_for(&workspace, &server.uri())).unwrap(),
    )
    .unwrap();
    let output = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "-s".into(),
            "metadata-opaque-key".into(),
            "-m".into(),
            "persist metadata".into(),
        ],
        &CliContext {
            base_dir: Some(base.path().to_path_buf()),
            cwd: Some(workspace.clone()),
            sessions: Some(quecto::composition::sessions::build_session_handles),
            retention: Some(quecto::composition::sessions::build_retention_handles),
            ..CliContext::default()
        },
    );
    assert_eq!(output.exit_code, 0, "real writer setup: {}", output.stderr);

    let canonical = workspace
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let records = authoritative_json_values(base.path());
    assert!(
        records.iter().any(|record| {
            value_contains_exact_string(record, "cli:metadata-opaque-key")
                && contains_versioned_home_subtree(record, &canonical)
        }),
        "the authoritative record must preserve its opaque key and carry optional home metadata whose subtree contains the canonical execution path plus an explicitly version-labelled string/number property; records={records:#?}"
    );
}

#[tokio::test]
async fn legacy_and_hostile_unknown_home_metadata_do_not_break_exact_key_loading() {
    let base = tempfile::tempdir().expect("temporary global store");
    let sessions = base.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join("cli_legacy.json"),
        serde_json::to_vec(&serde_json::json!({
            "key": "cli:legacy",
            "messages": [{"role": "user", "content": "old transcript"}]
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        sessions.join("cli_hostile.json"),
        serde_json::to_vec(&serde_json::json!({
            "key": "cli:hostile",
            "messages": [{"role": "user", "content": "still authoritative"}],
            "futureHomeMetadata": {
                "pathLikeText": "../../\u{0000}not-a-real-home/🚫",
                "unexpected": [true, {"deep": "value"}],
                "oversizedLabel": "x".repeat(16_384)
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let store = FileSessionStore::new(FlatSessionLayout::new(base.path()));
    for (key, expected) in [
        ("cli:legacy", "old transcript"),
        ("cli:hostile", "still authoritative"),
    ] {
        let loaded = store
            .load(&identity(key))
            .await
            .unwrap_or_else(|error| panic!("exact-key load of {key} failed: {error}"))
            .unwrap_or_else(|| panic!("exact key {key} became unavailable"));
        assert_eq!(loaded.key.persisted_key(), Some(key));
        assert_eq!(loaded.messages[0].content, expected);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_saves_and_global_lists_preserve_every_exact_opaque_key() {
    const SESSION_COUNT: usize = 24;
    let base = tempfile::tempdir().expect("temporary global store");
    let store: Arc<dyn SessionStore> =
        Arc::new(FileSessionStore::new(FlatSessionLayout::new(base.path())));
    let start = Arc::new(tokio::sync::Barrier::new(SESSION_COUNT + 2));
    let mut tasks = tokio::task::JoinSet::new();

    for number in 0..SESSION_COUNT {
        let store = store.clone();
        let start = start.clone();
        tasks.spawn(async move {
            start.wait().await;
            let key = format!("cli:concurrent-{number:02}");
            let mut session = Session::new(identity(&key));
            session
                .messages
                .push(Message::user(format!("message {number}")));
            store.save(&session).await.map(|()| key)
        });
    }

    let listing_store = store.clone();
    let listing_start = start.clone();
    tasks.spawn(async move {
        listing_start.wait().await;
        for _ in 0..SESSION_COUNT {
            listing_store.list(&SessionListQuery::All).await?;
            tokio::task::yield_now().await;
        }
        Ok::<String, quecto::domain::error::DomainError>("listing".into())
    });

    start.wait().await;
    while let Some(result) = tasks.join_next().await {
        result.expect("concurrent task did not panic").unwrap();
    }

    let listed = store.list(&SessionListQuery::All).await.unwrap();
    let listed_keys: std::collections::BTreeSet<_> =
        listed.iter().map(|summary| summary.key.as_str()).collect();
    for number in 0..SESSION_COUNT {
        let key = format!("cli:concurrent-{number:02}");
        assert!(listed_keys.contains(key.as_str()), "global list lost {key}");
        assert!(
            store.load(&identity(&key)).await.unwrap().is_some(),
            "exact-key recovery lost {key}"
        );
    }
}

const UDS_READY_BOUND: Duration = Duration::from_secs(20);

fn quecto_binary() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_quecto") {
        return PathBuf::from(path);
    }
    let test = std::env::current_exe().expect("current test executable");
    test.parent()
        .and_then(Path::parent)
        .expect("cargo target debug directory")
        .join("quecto")
}

fn write_uds_config(base: &Path, workspace: &Path) {
    std::fs::create_dir_all(workspace).expect("workspace fixture");
    std::fs::write(
        base.join("config.json"),
        serde_json::to_vec(&serde_json::json!({
            "providers": {"anthropic": {"api_key": "test-key-never-used"}},
            "agents": {"defaults": {
                "model": "anthropic-api/claude-test",
                "workspace": workspace
            }}
        }))
        .unwrap(),
    )
    .expect("UDS config fixture");
}

fn write_authoritative_session(base: &Path, key: &str, title: &str) {
    let directory = base.join("sessions");
    std::fs::create_dir_all(&directory).expect("authoritative sessions directory");
    let file = format!("{}.json", key.replace(':', "_"));
    std::fs::write(
        directory.join(file),
        serde_json::to_vec(&serde_json::json!({
            "key": key,
            "messages": [{"role": "user", "content": title}]
        }))
        .unwrap(),
    )
    .expect("authoritative session fixture");
}

struct UdsExchange {
    response: serde_json::Value,
    stderr: String,
}

fn wait_for_socket(child: &mut Child, socket: &Path) {
    let deadline = Instant::now() + UDS_READY_BOUND;
    while !socket.exists() {
        if let Some(status) = child.try_wait().expect("poll UDS process") {
            panic!(
                "UDS process exited before binding {}: {status}",
                socket.display()
            );
        }
        assert!(
            Instant::now() < deadline,
            "UDS process did not bind {}",
            socket.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Drive a raw, technically-approved request through the real UDS process.
/// Unknown additive request fields are parsed by today's command decoder, so
/// failures occur in response semantics rather than missing Rust symbols.
fn uds_exchange(
    base: &Path,
    workspace: &Path,
    client_name: &str,
    request: serde_json::Value,
) -> UdsExchange {
    let socket = base.join(format!("{client_name}.sock"));
    let mut child = Command::new(quecto_binary())
        .args(["agent", "--mode", "uds", "-s", client_name, "--socket"])
        .arg(&socket)
        .current_dir(workspace)
        .env("QUECTO_BASE_DIR", base)
        .env("HOME", base)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn real UDS process");
    wait_for_socket(&mut child, &socket);

    let mut stream = UnixStream::connect(&socket).expect("connect to real UDS socket");
    stream
        .set_read_timeout(Some(UDS_READY_BOUND))
        .expect("bound UDS response wait");
    writeln!(stream, "{request}").expect("write UDS request");
    let expected_id = request["id"].as_str().expect("correlated request id");
    let mut lines = Vec::new();
    let response = {
        let mut reader = BufReader::new(&stream);
        loop {
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .expect("read bounded UDS response line");
            assert!(
                !line.is_empty(),
                "UDS closed before response; lines={lines:?}"
            );
            let line = line.trim_end().to_owned();
            let parsed = serde_json::from_str::<serde_json::Value>(&line).ok();
            lines.push(line);
            if let Some(event) = parsed
                && event["type"] == "response"
                && event["id"] == expected_id
            {
                break event;
            }
        }
    };
    stream
        .shutdown(std::net::Shutdown::Both)
        .expect("close UDS client after correlated response");
    drop(stream);
    let output = child.wait_with_output().expect("join real UDS process");
    assert!(
        output.status.success(),
        "UDS fixture exits cleanly: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    UdsExchange {
        response,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn global_list(base: &Path, workspace: &Path, client_name: &str) -> UdsExchange {
    uds_exchange(
        base,
        workspace,
        client_name,
        serde_json::json!({
            "type": "list_sessions",
            "id": format!("list-{client_name}"),
            "scope": "global"
        }),
    )
}

fn response_sessions(exchange: &UdsExchange) -> &[serde_json::Value] {
    assert_eq!(
        exchange.response["success"],
        serde_json::Value::Bool(true),
        "normal global discovery must succeed: {}",
        exchange.response
    );
    exchange.response["data"]["sessions"]
        .as_array()
        .expect("successful list_sessions data.sessions")
}

fn row<'a>(sessions: &'a [serde_json::Value], key: &str) -> &'a serde_json::Value {
    sessions
        .iter()
        .find(|row| row["key"] == key)
        .unwrap_or_else(|| panic!("missing authoritative key {key}; rows={sessions:#?}"))
}

#[test]
fn missing_derived_index_is_built_or_bypassed_by_normal_global_discovery() {
    let base = tempfile::tempdir().unwrap();
    let workspace = base.path().join("workspace");
    write_uds_config(base.path(), &workspace);
    write_authoritative_session(base.path(), "cli:missing-index", "authority survives");

    let exchange = global_list(base.path(), &workspace, "missing-index-client");
    let found = row(response_sessions(&exchange), "cli:missing-index");
    assert_eq!(found["title"], "authority survives");
}

#[test]
fn stale_discovery_is_repaired_from_the_changed_authoritative_transcript() {
    let base = tempfile::tempdir().unwrap();
    let workspace = base.path().join("workspace");
    write_uds_config(base.path(), &workspace);
    write_authoritative_session(base.path(), "cli:stale", "old indexed title");
    let first = global_list(base.path(), &workspace, "stale-before");
    assert_eq!(
        row(response_sessions(&first), "cli:stale")["title"],
        "old indexed title"
    );

    write_authoritative_session(base.path(), "cli:stale", "new authoritative title");
    let repaired = global_list(base.path(), &workspace, "stale-after");
    assert_eq!(
        row(response_sessions(&repaired), "cli:stale")["title"],
        "new authoritative title",
        "normal discovery must not retain a stale derived row"
    );
}

#[test]
fn malformed_authoritative_record_is_diagnosed_without_hiding_valid_sessions() {
    let base = tempfile::tempdir().unwrap();
    let workspace = base.path().join("workspace");
    write_uds_config(base.path(), &workspace);
    write_authoritative_session(base.path(), "cli:healthy", "healthy transcript");
    std::fs::write(base.path().join("sessions/broken.json"), b"{not json")
        .expect("malformed authoritative fixture");

    let exchange = global_list(base.path(), &workspace, "bad-record-client");
    assert_eq!(
        row(response_sessions(&exchange), "cli:healthy")["title"],
        "healthy transcript"
    );
    let diagnostic = exchange.stderr.to_lowercase();
    assert!(
        diagnostic.contains("broken.json")
            && (diagnostic.contains("invalid") || diagnostic.contains("malformed")),
        "a bad record must produce a sanitized, actionable diagnostic while healthy rows remain available; stderr={}",
        exchange.stderr
    );
    assert!(
        !exchange.stderr.contains("{not json"),
        "diagnostics must not reflect hostile record content"
    );
}

#[test]
fn repeated_discovery_remains_correct_when_no_persisted_derived_index_is_observable() {
    let base = tempfile::tempdir().unwrap();
    let workspace = base.path().join("workspace");
    write_uds_config(base.path(), &workspace);
    write_authoritative_session(base.path(), "cli:stateless", "authoritative title");

    let first = global_list(base.path(), &workspace, "stateless-before");
    row(response_sessions(&first), "cli:stateless");
    let second = global_list(base.path(), &workspace, "stateless-after");
    assert_eq!(
        row(response_sessions(&second), "cli:stateless")["title"],
        "authoritative title",
        "normal discovery may correctly bypass a persisted derived index"
    );
}

#[test]
fn concurrent_public_global_discovery_never_loses_authoritative_sessions() {
    const SESSION_COUNT: usize = 12;
    const CLIENT_COUNT: usize = 4;
    let base = tempfile::tempdir().unwrap();
    let workspace = base.path().join("workspace");
    write_uds_config(base.path(), &workspace);
    for number in 0..SESSION_COUNT {
        write_authoritative_session(
            base.path(),
            &format!("cli:public-{number:02}"),
            &format!("public title {number}"),
        );
    }

    std::thread::scope(|scope| {
        let mut clients = Vec::new();
        let base_path = base.path();
        for client in 0..CLIENT_COUNT {
            let workspace = &workspace;
            clients.push(
                scope.spawn(move || {
                    global_list(base_path, workspace, &format!("parallel-{client}"))
                }),
            );
        }
        for client in clients {
            let exchange = client.join().expect("public discovery client");
            let sessions = response_sessions(&exchange);
            for number in 0..SESSION_COUNT {
                row(sessions, &format!("cli:public-{number:02}"));
            }
        }
    });
}

#[test]
fn approved_global_query_filters_title_and_opaque_key_through_public_uds() {
    let base = tempfile::tempdir().unwrap();
    let workspace = base.path().join("workspace");
    write_uds_config(base.path(), &workspace);
    write_authoritative_session(base.path(), "cli:needle-key", "alpha title needle");
    write_authoritative_session(base.path(), "cli:other-key", "unrelated title");

    for (client, query) in [("query-title", "alpha title"), ("query-key", "needle-key")] {
        let exchange = uds_exchange(
            base.path(),
            &workspace,
            client,
            serde_json::json!({
                "type": "list_sessions",
                "id": format!("request-{client}"),
                "scope": "global",
                "query": query
            }),
        );
        let sessions = response_sessions(&exchange);
        row(sessions, "cli:needle-key");
        assert!(
            sessions
                .iter()
                .all(|candidate| candidate["key"] != "cli:other-key"),
            "approved metadata query {query:?} must filter non-matches; rows={sessions:#?}"
        );
    }
}

#[test]
fn approved_global_path_query_exposes_canonical_execution_directory_without_changing_key() {
    let runtime = tokio::runtime::Runtime::new().expect("test runtime");
    let server = runtime.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": "saved"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            })))
            .mount(&server)
            .await;
        server
    });
    let base = tempfile::tempdir().unwrap();
    let workspace = base
        .path()
        .join("repository-label")
        .join("actual-execution-directory");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        base.path().join("config.json"),
        serde_json::to_vec(&config_for(&workspace, &server.uri())).unwrap(),
    )
    .unwrap();
    let context = CliContext {
        base_dir: Some(base.path().to_path_buf()),
        cwd: Some(workspace.clone()),
        sessions: Some(quecto::composition::sessions::build_session_handles),
        retention: Some(quecto::composition::sessions::build_retention_handles),
        ..CliContext::default()
    };
    let saved = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "-s".into(),
            "path-search".into(),
            "-m".into(),
            "title without path tokens".into(),
        ],
        &context,
    );
    assert_eq!(saved.exit_code, 0, "real writer setup: {}", saved.stderr);

    let canonical = workspace
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let observations: Vec<_> = [
        ("path-query-client", "actual-execution-directory"),
        ("repository-query-client", "repository-label"),
    ]
    .into_iter()
    .map(|(client, query)| {
        let exchange = uds_exchange(
            base.path(),
            &workspace,
            client,
            serde_json::json!({
                "type": "list_sessions", "id": format!("request-{client}"),
                "scope": "global", "query": query
            }),
        );
        let found = row(response_sessions(&exchange), "cli:path-search").clone();
        (query, found)
    })
    .collect();
    assert!(
        observations
            .iter()
            .all(|(_, found)| value_contains_exact_string(found, &canonical)),
        "global path and repository-label metadata searches must both return sanitized display metadata that visibly distinguishes the canonical execution directory without changing the opaque key; observations={observations:#?}"
    );
}

#[test]
fn global_metadata_query_does_not_search_transcript_body_content() {
    let base = tempfile::tempdir().unwrap();
    let workspace = base.path().join("workspace");
    write_uds_config(base.path(), &workspace);
    let sessions = base.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join("cli_transcript_witness.json"),
        serde_json::to_vec(&serde_json::json!({
            "key": "cli:transcript-witness",
            "messages": [
                {"role": "user", "content": "ordinary visible title"},
                {"role": "assistant", "content": "body-only-secret-needle"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let exchange = uds_exchange(
        base.path(),
        &workspace,
        "transcript-query-client",
        serde_json::json!({
            "type": "list_sessions", "id": "transcript-query",
            "scope": "global", "query": "body-only-secret-needle"
        }),
    );
    let sessions = response_sessions(&exchange);
    assert!(
        sessions
            .iter()
            .all(|candidate| candidate["key"] != "cli:transcript-witness"),
        "the approved metadata-only query must not become transcript-content search; rows={sessions:#?}"
    );
}

#[test]
fn malformed_sibling_and_derived_recovery_never_disable_exact_key_resume() {
    let base = tempfile::tempdir().unwrap();
    let workspace = base.path().join("workspace");
    write_uds_config(base.path(), &workspace);
    write_authoritative_session(base.path(), "cli:exact-authority", "exact transcript");
    std::fs::write(base.path().join("sessions/malformed.json"), b"not-json").unwrap();

    let exchange = uds_exchange(
        base.path(),
        &workspace,
        "exact-resume-client",
        serde_json::json!({
            "type": "resume_session", "id": "resume-exact",
            "session": "cli:exact-authority"
        }),
    );
    assert_eq!(
        exchange.response["success"], true,
        "{:?}",
        exchange.response
    );
    assert!(
        exchange.response["data"]["session"] == "cli:exact-authority"
            || exchange.response["data"]["sessionKey"] == "cli:exact-authority",
        "exact opaque-key authority must remain globally resumable despite malformed siblings or derived recovery: {}",
        exchange.response
    );
}
