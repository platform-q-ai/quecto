//! The effect port is object safe, Send, and preserves owned neutral outcomes.
use quecto::application::agent_turn::use_cases::find::{
    FindError, FindOutput, FindPaths, FindPathsRequest, FindRequest, FindUseCase,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

struct RecordingPaths {
    calls: Mutex<Vec<FindPathsRequest>>,
    outcome: Mutex<Option<Result<FindOutput, FindError>>>,
}

impl FindPaths for RecordingPaths {
    fn find(
        &self,
        request: FindPathsRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FindOutput, FindError>> + Send + '_>> {
        self.calls.lock().unwrap().push(request);
        let outcome = self
            .outcome
            .lock()
            .unwrap()
            .take()
            .expect("exactly one effect invocation");
        Box::pin(async move { outcome })
    }
}

#[tokio::test]
async fn typed_port_preserves_incomplete_output_and_normalized_request() {
    let effect = Arc::new(RecordingPaths {
        calls: Mutex::new(Vec::new()),
        outcome: Mutex::new(Some(Ok(FindOutput {
            entries: vec!["nested/δ file.rs".into(), "directory/".into()],
            result_limit_reached: true,
            incomplete: true,
            diagnostic: Some("bounded discovery".into()),
        }))),
    });
    let port: Arc<dyn FindPaths> = effect.clone();
    let result = FindUseCase::new(port)
        .execute(FindRequest {
            pattern: "**/*.rs".into(),
            path: "root".into(),
            limit: Some(5.5),
        })
        .await
        .unwrap();
    assert_eq!(result.limit, 6);
    assert_eq!(result.output.entries, ["nested/δ file.rs", "directory/"]);
    assert!(result.output.result_limit_reached);
    assert!(result.output.incomplete);
    assert_eq!(
        result.output.diagnostic.as_deref(),
        Some("bounded discovery")
    );
    let calls = effect.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].pattern, "**/*.rs");
    assert_eq!(calls[0].path, "root");
    assert_eq!(calls[0].limit, 6);
}

#[tokio::test]
async fn typed_port_preserves_each_failure_category() {
    for error in [
        FindError::Security("security".into()),
        FindError::Spawn("spawn".into()),
        FindError::Search("search".into()),
        FindError::Io("io".into()),
    ] {
        let expected = format!("{error:?}");
        let effect = Arc::new(RecordingPaths {
            calls: Mutex::new(Vec::new()),
            outcome: Mutex::new(Some(Err(error))),
        });
        let port: Arc<dyn FindPaths> = effect.clone();
        let actual = FindUseCase::new(port)
            .execute(FindRequest {
                pattern: "*".into(),
                path: ".".into(),
                limit: None,
            })
            .await
            .unwrap_err();
        assert_eq!(format!("{actual:?}"), expected);
        assert_eq!(effect.calls.lock().unwrap().len(), 1);
    }
}
