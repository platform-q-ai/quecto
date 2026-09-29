//! The refusal-site scan reads texts, templates and kinds as the table
//! names them, and counts production sources only.
use super::{built, production};

#[test]
fn the_scan_reads_texts_templates_and_kinds() {
    let found = built(
        r#"fn f() {
            let _ = BoardError::new(RefusalKind::Invalid, "plain");
            let _ = BoardError::new(RefusalKind::NotRunning, format!("run is {}; no", x));
            let _ = crate::domain::swarm::BoardError::new(kind, error.to_string());
            let _ = StoreRefusal(m);
        }
        impl Store {
            fn g(&self) {
                let _ = || BoardError::new(RefusalKind::Invalid, "plain");
                fn inner() { let _ = BoardError::new(RefusalKind::Invalid, "plain"); }
            }
        }"#,
    );
    let site = |function: &str, text: &str, kind: &str| {
        (function.to_owned(), text.to_owned(), kind.to_owned())
    };
    assert_eq!(
        found,
        [
            site("f", "plain", "Invalid"),
            site("f", "run is {}; no", "NotRunning"),
            site("f", "expr: error . to_string ()", "expr: kind"),
            site("Store::g", "plain", "Invalid"),
            site("inner", "plain", "Invalid"),
        ]
    );
    assert!(production("src/domain/swarm/policy.rs"));
    assert!(!production("src/domain/swarm/telemetry_tests.rs"));
    assert!(!production("src/application/swarm/board_test_support.rs"));
    // An allowlist (#2303 round-3 review L5): a file the production module
    // tree does not mount is not production, whatever its name.
    assert!(!production("src/nowhere.rs"));
    assert!(!production(
        "src/application/swarm/use_cases/create_run_tests.rs"
    ));
}
