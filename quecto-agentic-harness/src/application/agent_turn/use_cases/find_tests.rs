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
                kind: None,
            })
            .await
            .unwrap();
        assert_eq!(result.limit, expected);
        assert_eq!(
            *fake.calls.lock().unwrap(),
            vec![FindPathsRequest {
                pattern: "".into(),
                path: "somewhere".into(),
                limit: expected,
                kind: None,
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
        skipped_vcs_dir: None,
    };
    let fake = Arc::new(Fake::default());
    *fake.response.lock().unwrap() = Some(Ok(output.clone()));
    let use_case = FindUseCase::new(fake.clone());
    let request = || FindRequest {
        pattern: "*".into(),
        path: ".".into(),
        limit: None,
        kind: None,
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

fn request(pattern: &str, kind: Option<FindEntryKind>) -> FindRequest {
    FindRequest {
        pattern: pattern.into(),
        path: ".".into(),
        limit: None,
        kind,
    }
}

/// #2200: the entry kind asked for reaches the effect and the result.
#[tokio::test]
async fn the_entry_kind_passes_through() {
    for kind in [
        None,
        Some(FindEntryKind::File),
        Some(FindEntryKind::Directory),
    ] {
        let fake = Arc::new(Fake::default());
        let result = FindUseCase::new(fake.clone())
            .execute(request("*.rs", kind))
            .await
            .unwrap();
        assert_eq!(result.kind, kind);
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls[0].kind, kind);
        assert_eq!(calls[0].pattern, "*.rs");
    }
}

/// #2200: a pattern ending in '/' means directories matching the rest; it
/// never reaches fd, where it could match nothing.
#[tokio::test]
async fn a_trailing_slash_means_directories() {
    for (pattern, expected) in [
        ("*/", "*"),
        ("src/*/", "src/*"),
        ("src//", "src"),
        ("/", ""),
        // `./` is every directory here, never the name '.' (review 3).
        ("./", ""),
        (".//", ""),
        ("././", ""),
        ("src/./", "src"),
        ("./src/", "./src"),
    ] {
        for kind in [None, Some(FindEntryKind::Directory)] {
            let fake = Arc::new(Fake::default());
            let result = FindUseCase::new(fake.clone())
                .execute(request(pattern, kind))
                .await
                .unwrap();
            assert_eq!(result.kind, Some(FindEntryKind::Directory), "{pattern}");
            let calls = fake.calls.lock().unwrap();
            assert_eq!(calls[0].pattern, expected, "{pattern}");
            assert_eq!(calls[0].kind, Some(FindEntryKind::Directory), "{pattern}");
        }
    }
}

#[tokio::test]
async fn a_trailing_slash_with_type_file_is_refused_before_any_search() {
    let fake = Arc::new(Fake::default());
    let error = FindUseCase::new(fake.clone())
        .execute(request("*/", Some(FindEntryKind::File)))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, FindError::Search(message) if message.contains("ends in '/'") && message.contains("type \"f\"")),
        "{error:?}"
    );
    assert!(fake.calls.lock().unwrap().is_empty());
}
