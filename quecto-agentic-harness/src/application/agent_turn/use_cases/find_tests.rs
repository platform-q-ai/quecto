use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<FindPathsRequest>>,
    response: Mutex<Option<Result<FindOutput, FindError>>>,
}
impl FindPaths for Fake {
    fn find(
        &self,
        request: FindPathsRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FindOutput, FindError>> + Send + '_>> {
        self.calls.lock().unwrap().push(request);
        Box::pin(async {
            self.response
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Ok(FindOutput::default()))
        })
    }
}

#[tokio::test]
async fn normalizes_limits_and_calls_effect_exactly_once() {
    for (input, expected) in [
        (None, 1000),
        (Some(0.5), 1),
        (Some(5.4), 5),
        (Some(5.5), 6),
        (Some(-8.0), 1),
        (Some(0.0), 1),
        (Some(1e99), 100000),
        (Some(100000.0), 100000),
        (Some(f64::NAN), 1),
        (Some(f64::INFINITY), 100000),
    ] {
        let fake = Arc::new(Fake::default());
        let result = FindUseCase::new(fake.clone())
            .execute(FindRequest {
                pattern: "".into(),
                path: "somewhere".into(),
                limit: input,
            })
            .await
            .unwrap();
        assert_eq!(result.limit, expected);
        assert_eq!(
            *fake.calls.lock().unwrap(),
            vec![FindPathsRequest {
                pattern: "".into(),
                path: "somewhere".into(),
                limit: expected
            }]
        );
    }
}

#[tokio::test]
async fn preserves_typed_output_and_every_failure() {
    let output = FindOutput {
        entries: vec!["dir/".into(), "文.rs".into()],
        result_limit_reached: true,
        incomplete: true,
        diagnostic: Some("partial".into()),
    };
    let fake = Arc::new(Fake::default());
    *fake.response.lock().unwrap() = Some(Ok(output.clone()));
    let use_case = FindUseCase::new(fake.clone());
    let request = || FindRequest {
        pattern: "*".into(),
        path: ".".into(),
        limit: None,
    };
    assert_eq!(use_case.execute(request()).await.unwrap().output, output);
    for error in [
        FindError::Security("s".into()),
        FindError::Spawn("p".into()),
        FindError::Search("b".into()),
        FindError::Io("i".into()),
    ] {
        *fake.response.lock().unwrap() = Some(Err(error.clone()));
        assert_eq!(use_case.execute(request()).await.unwrap_err(), error);
    }
    assert_eq!(fake.calls.lock().unwrap().len(), 5);
}
