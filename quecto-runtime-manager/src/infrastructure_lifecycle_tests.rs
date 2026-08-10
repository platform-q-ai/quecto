use super::*;

fn ensure_request(model: Option<&str>) -> EnsureRuntimeRequest {
    let mut request = board_pod_request();
    request.execution_model = model.map(str::to_string);
    request
}

async fn post_ensure(
    app: Router,
    token: &str,
    request: &EnsureRuntimeRequest,
) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/runtimes/ensure")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(request).unwrap()))
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn ensure_runtime_create_and_duplicate_reuse_call_launcher_once() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle::default());
    let starts = fake.starts.clone();
    let state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), fake);
    let request = ensure_request(Some("process"));

    let first = post_ensure(router(state.clone()), "secret", &request).await;
    let second = post_ensure(router(state), "secret", &request).await;

    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(starts.lock().await.len(), 1);
}

#[tokio::test]
async fn concurrent_duplicate_ensure_waits_for_pending_start_without_second_launch() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle {
        delay_start: true,
        ..Default::default()
    });
    let starts = fake.starts.clone();
    let state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), fake);
    let request = ensure_request(Some("process"));

    let (first, second) = tokio::join!(
        post_ensure(router(state.clone()), "secret", &request),
        post_ensure(router(state), "secret", &request)
    );
    let statuses = [first.status(), second.status()];

    assert!(statuses.contains(&StatusCode::CREATED));
    assert!(statuses.contains(&StatusCode::OK));
    assert_eq!(starts.lock().await.len(), 1);
}

#[tokio::test]
async fn ensure_runtime_rejects_auth_and_invalid_request_without_launching() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle::default());
    let starts = fake.starts.clone();
    let state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), fake);
    let app = router(state);

    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/runtimes/ensure")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&ensure_request(Some("process"))).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let mut invalid = ensure_request(Some("process"));
    invalid.session_name.clear();
    let invalid_response = post_ensure(app, "secret", &invalid).await;

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(invalid_response.status(), StatusCode::BAD_REQUEST);
    assert!(starts.lock().await.is_empty());
}

#[tokio::test]
async fn ensure_runtime_process_and_pod_delegate_through_lifecycle_seam() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle::default());
    let starts = fake.starts.clone();
    let state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), fake);

    let process = post_ensure(
        router(state.clone()),
        "secret",
        &ensure_request(Some("process")),
    )
    .await;
    let mut pod_request = ensure_request(Some("pod"));
    pod_request.chat_id = "different-run".to_string();
    let pod = post_ensure(router(state), "secret", &pod_request).await;

    assert_eq!(process.status(), StatusCode::CREATED);
    assert_eq!(pod.status(), StatusCode::CREATED);
    let starts = starts.lock().await;
    assert!(starts.iter().any(|(_, model, _)| model == "process"));
    assert!(starts.iter().any(|(_, model, _)| model == "pod"));
}

#[tokio::test]
async fn stop_runtime_deletes_registered_pod_once() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle::default());
    let deletes = fake.pod_deletes.clone();
    let state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), fake);
    let mut runtime = fake_runtime("cc-pod", tmp.path().join("pod.sock"));
    runtime.pod_name = Some("quecto-runtime-cc-pod".to_string());
    state.registry.lock().await.insert(runtime);

    let first = router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/runtimes/cc-pod")
                .header("authorization", "Bearer secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let second = router(state)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/runtimes/cc-pod")
                .header("authorization", "Bearer secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        *deletes.lock().await,
        vec!["quecto-runtime-cc-pod".to_string()]
    );
}

#[tokio::test]
async fn sync_credentials_validates_payloads_before_calling_store() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle::default());
    let syncs = fake.credential_syncs.clone();
    let app = router(test_state_with_lifecycle(
        &tmp,
        Some("secret".to_string()),
        fake,
    ));

    for body in [
        r#"{}"#,
        r#"{"credentials_json":42}"#,
        r#"{"credentials_json":"not json"}"#,
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/credentials")
                    .header("authorization", "Bearer secret")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/credentials")
                .header("authorization", "Bearer secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"credentials_json":"{\"token\":\"fresh\"}"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        *syncs.lock().await,
        vec![r#"{"token":"fresh"}"#.to_string()]
    );
}

#[tokio::test]
async fn ensure_runtime_capacity_reap_deletes_oldest_registered_pod() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle::default());
    let deletes = fake.pod_deletes.clone();
    let mut state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), fake);
    Arc::get_mut(&mut state.config).unwrap().max_runtimes = 2;
    let mut old = fake_runtime("cc-old", tmp.path().join("old.sock"));
    old.pod_name = Some("quecto-runtime-cc-old".to_string());
    old.last_used_at = Instant::now() - Duration::from_secs(60);
    let mut newer = fake_runtime("cc-newer", tmp.path().join("newer.sock"));
    newer.pod_name = Some("quecto-runtime-cc-newer".to_string());
    state.registry.lock().await.insert(old);
    state.registry.lock().await.insert(newer);
    let mut request = ensure_request(Some("pod"));
    request.chat_id = "new-chat".to_string();

    let response = post_ensure(router(state.clone()), "secret", &request).await;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        *deletes.lock().await,
        vec!["quecto-runtime-cc-old".to_string()]
    );
    assert!(state.registry.lock().await.get("cc-newer").is_some());
}

#[tokio::test]
async fn runtime_status_is_unauthenticated_and_delegates_registered_pod_only() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle::default());
    let statuses = fake.pod_statuses.clone();
    let state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), fake);
    state
        .registry
        .lock()
        .await
        .insert(fake_runtime("cc-process", tmp.path().join("process.sock")));
    let mut pod = fake_runtime("cc-pod", tmp.path().join("pod.sock"));
    pod.pod_name = Some("quecto-runtime-cc-pod".to_string());
    state.registry.lock().await.insert(pod);
    let app = router(state);

    let process = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/runtimes/cc-process/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let pod = app
        .oneshot(
            Request::builder()
                .uri("/runtimes/cc-pod/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(process.status(), StatusCode::NOT_FOUND);
    assert_eq!(pod.status(), StatusCode::OK);
    assert_eq!(
        *statuses.lock().await,
        vec!["quecto-runtime-cc-pod".to_string()]
    );
}

#[derive(Clone)]
struct FailingLifecycle {
    starts: Arc<Mutex<usize>>,
    fail_credentials: bool,
    fail_start: bool,
    fail_status: bool,
}

impl RuntimeLifecycle for FailingLifecycle {
    fn start_runtime(
        &self,
        state: AppState,
        body: EnsureRuntimeRequest,
        runtime_ref: String,
        port: u16,
    ) -> BoxFutureResult<ManagedRuntime> {
        let starts = self.starts.clone();
        let fail_start = self.fail_start;
        Box::pin(async move {
            *starts.lock().await += 1;
            if fail_start {
                return Err(ManagerError::InvalidRequest("boom".to_string()));
            }
            ProductionRuntimeLifecycle
                .start_runtime(state, body, runtime_ref, port)
                .await
        })
    }

    fn sync_credentials(&self, _state: AppState, _credentials_json: String) -> BoxFutureResult<()> {
        let fail_credentials = self.fail_credentials;
        Box::pin(async move {
            if fail_credentials {
                Err(ManagerError::KubernetesApi(500))
            } else {
                Ok(())
            }
        })
    }

    fn delete_runtime_pod(&self, _state: AppState, _pod_name: String) -> BoxFutureResult<()> {
        Box::pin(async move { Ok(()) })
    }

    fn runtime_pod_status(&self, _state: AppState, pod_name: String) -> BoxFutureResult<Value> {
        let fail_status = self.fail_status;
        Box::pin(async move {
            if fail_status {
                Err(ManagerError::KubernetesApi(503))
            } else {
                Ok(json!({ "pod_name": pod_name }))
            }
        })
    }
}

#[test]
fn runtime_pod_name_preserves_hash_suffix_when_truncating() {
    let first = runtime_pod_name("cc-verylongagent-verylongproj-verylongchat-1111111111111111");
    let second = runtime_pod_name("cc-verylongagent-verylongproj-verylongchat-2222222222222222");

    assert!(first.len() <= 63);
    assert!(second.len() <= 63);
    assert!(first.ends_with("1111111111111111"));
    assert!(second.ends_with("2222222222222222"));
    assert_ne!(first, second);
}

#[test]
fn runtime_pod_name_caps_long_or_malformed_refs_at_dns_label_limit() {
    let without_hyphen = runtime_pod_name(&"a".repeat(100));
    let long_suffix = runtime_pod_name(&format!("cc-short-{}", "b".repeat(100)));

    assert!(without_hyphen.len() <= 63);
    assert!(long_suffix.len() <= 63);
}

#[tokio::test]
async fn sync_credentials_store_failure_returns_500() {
    let tmp = tempfile::tempdir().unwrap();
    let lifecycle = Arc::new(FailingLifecycle {
        starts: Arc::new(Mutex::new(0)),
        fail_credentials: true,
        fail_start: false,
        fail_status: false,
    });
    let app = router(test_state_with_lifecycle(
        &tmp,
        Some("secret".to_string()),
        lifecycle,
    ));

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/credentials")
                .header("authorization", "Bearer secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"credentials_json":"{\"token\":\"fresh\"}"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn failed_start_clears_pending_and_allows_retry() {
    let tmp = tempfile::tempdir().unwrap();
    let starts = Arc::new(Mutex::new(0));
    let failing = Arc::new(FailingLifecycle {
        starts: starts.clone(),
        fail_credentials: false,
        fail_start: true,
        fail_status: false,
    });
    let mut state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), failing);
    let request = ensure_request(Some("process"));

    let failed = post_ensure(router(state.clone()), "secret", &request).await;
    state.lifecycle = Arc::new(FakeLifecycle::default());
    let retry = post_ensure(router(state), "secret", &request).await;

    assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(retry.status(), StatusCode::CREATED);
    assert_eq!(*starts.lock().await, 1);
}

#[tokio::test]
async fn ensure_runtime_returns_503_when_no_port_is_available_and_clears_pending() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle::default());
    let mut state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), fake.clone());
    Arc::get_mut(&mut state.config).unwrap().api_port_span = 1;
    state.registry.lock().await.reserve_port_for_test(21000);
    let request = ensure_request(Some("process"));

    let failed = post_ensure(router(state.clone()), "secret", &request).await;
    state.registry.lock().await.release_port(21000);
    let retry = post_ensure(router(state), "secret", &request).await;

    assert_eq!(failed.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(retry.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn wrong_auth_rejects_mutating_routes_without_lifecycle_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle::default());
    let starts = fake.starts.clone();
    let syncs = fake.credential_syncs.clone();
    let deletes = fake.pod_deletes.clone();
    let state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), fake);
    let mut runtime = fake_runtime("cc-auth", tmp.path().join("auth.sock"));
    runtime.pod_name = Some("quecto-runtime-cc-auth".to_string());
    state.registry.lock().await.insert(runtime);
    let app = router(state);

    let ensure = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/runtimes/ensure")
                .header("authorization", "Bearer wrong")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&ensure_request(Some("process"))).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let stop = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/runtimes/cc-auth")
                .header("authorization", "Bearer wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let credentials = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/credentials")
                .header("x-quecto-token", "wrong")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"credentials_json":"{}"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(ensure.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(stop.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(credentials.status(), StatusCode::UNAUTHORIZED);
    assert!(starts.lock().await.is_empty());
    assert!(syncs.lock().await.is_empty());
    assert!(deletes.lock().await.is_empty());
}

#[tokio::test]
async fn runtime_status_lifecycle_failure_returns_bad_gateway() {
    let tmp = tempfile::tempdir().unwrap();
    let lifecycle = Arc::new(FailingLifecycle {
        starts: Arc::new(Mutex::new(0)),
        fail_credentials: false,
        fail_start: false,
        fail_status: true,
    });
    let state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), lifecycle);
    let mut pod = fake_runtime("cc-pod", tmp.path().join("pod.sock"));
    pod.pod_name = Some("quecto-runtime-cc-pod".to_string());
    state.registry.lock().await.insert(pod);

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/runtimes/cc-pod/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn concurrent_distinct_ensure_without_active_capacity_rejects_extra_pending_start() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle {
        delay_start: true,
        ..Default::default()
    });
    let mut state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), fake);
    Arc::get_mut(&mut state.config).unwrap().max_runtimes = 1;
    let first = ensure_request(Some("process"));
    let mut second = ensure_request(Some("process"));
    second.chat_id = "second-chat".to_string();

    let (first_response, second_response) = tokio::join!(
        post_ensure(router(state.clone()), "secret", &first),
        post_ensure(router(state.clone()), "secret", &second)
    );
    let statuses = [first_response.status(), second_response.status()];

    assert!(statuses.contains(&StatusCode::CREATED));
    assert!(statuses.contains(&StatusCode::SERVICE_UNAVAILABLE));
    assert_eq!(state.registry.lock().await.active_count(), 1);
}

#[tokio::test]
async fn concurrent_distinct_ensure_can_reap_old_active_capacity_for_burst() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeLifecycle {
        delay_start: true,
        ..Default::default()
    });
    let mut state = test_state_with_lifecycle(&tmp, Some("secret".to_string()), fake);
    Arc::get_mut(&mut state.config).unwrap().max_runtimes = 2;
    let mut old = fake_runtime("cc-old", tmp.path().join("old.sock"));
    old.last_used_at = Instant::now() - Duration::from_secs(60);
    state.registry.lock().await.insert(old);
    let first = ensure_request(Some("process"));
    let mut second = ensure_request(Some("process"));
    second.chat_id = "second-chat".to_string();

    let (first_response, second_response) = tokio::join!(
        post_ensure(router(state.clone()), "secret", &first),
        post_ensure(router(state.clone()), "secret", &second)
    );

    assert_eq!(first_response.status(), StatusCode::CREATED);
    assert_eq!(second_response.status(), StatusCode::CREATED);
    let registry = state.registry.lock().await;
    assert_eq!(registry.active_count(), 2);
    assert!(registry.get("cc-old").is_none());
}
