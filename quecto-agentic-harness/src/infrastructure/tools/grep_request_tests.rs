use super::*;
use serde_json::json;

fn parsed(args: Value) -> GrepRequest {
    parse_request(&args).unwrap_or_else(|e| panic!("{args} should parse: {e}"))
}

fn refused(args: Value) -> String {
    parse_request(&args).expect_err(&format!("{args} should be refused"))
}

#[test]
fn a_bare_pattern_takes_every_default() {
    assert_eq!(
        parsed(json!({"pattern": "needle"})),
        GrepRequest {
            patterns: vec!["needle".into()],
            path: ".".into(),
            globs: vec![],
            types: vec![],
            ignore_case: false,
            literal: false,
            word: false,
            multiline: false,
            max_per_file: None,
            output: OutputMode::Content,
            context_lines: 0,
            limit: DEFAULT_MATCH_LIMIT,
        }
    );
}

#[test]
fn every_rg_option_is_carried() {
    let request = parsed(json!({
        "pattern": "a", "patterns": ["b", "c"], "path": "src",
        "glob": ["*.rs", "!*_tests.rs"], "type": "rust",
        "ignoreCase": true, "literal": true, "wordRegexp": true, "multiline": true,
        "maxPerFile": 2, "output": "count", "context": 3, "limit": 7.4
    }));
    assert_eq!(request.patterns, ["a", "b", "c"]);
    assert_eq!(request.path, "src");
    assert_eq!(request.globs, ["*.rs", "!*_tests.rs"]);
    assert_eq!(request.types, ["rust"]);
    assert!(request.ignore_case && request.literal && request.word && request.multiline);
    assert_eq!(request.max_per_file, Some(2));
    assert_eq!(request.output, OutputMode::Count);
    assert_eq!(request.context_lines, 3);
    assert_eq!(request.limit, 7, "models send floats: rounded");
    assert_eq!(
        parsed(json!({"patterns": ["x"], "output": "files"})).output,
        OutputMode::Files
    );
    assert_eq!(
        parsed(json!({"pattern": "x", "context": 500})).context_lines,
        MAX_CONTEXT_LINES
    );
    assert_eq!(
        parsed(json!({"pattern": "x", "context": 0})).context_lines,
        0
    );
}

#[test]
fn a_search_for_nothing_is_refused() {
    for args in [
        json!({}),
        json!({"pattern": ""}),
        json!({"patterns": []}),
        json!({"patterns": ["ok", ""]}),
    ] {
        assert!(refused(args.clone()).contains("pattern"), "{args}");
    }
    assert!(refused(json!({"pattern": 3})).contains("pattern must be a string"));
    assert!(refused(json!({"patterns": "x"})).contains("patterns must be an array"));
}

#[test]
fn an_argument_of_the_wrong_shape_is_refused_by_name() {
    let cases = [
        (
            json!({"pattern": "x", "output": "lines"}),
            "output must be one of content, files, count (got 'lines')",
        ),
        (
            json!({"pattern": "x", "output": 1}),
            "output must be one of content, files, count",
        ),
        (
            json!({"pattern": "x", "glob": ""}),
            "glob must be a non-empty string",
        ),
        (
            json!({"pattern": "x", "glob": ["*.rs", 3]}),
            "glob must hold only non-empty strings",
        ),
        (
            json!({"pattern": "x", "type": true}),
            "type must be a non-empty string",
        ),
        (
            json!({"pattern": "x", "wordRegexp": "yes"}),
            "wordRegexp must be true or false",
        ),
        (
            json!({"pattern": "x", "maxPerFile": 0}),
            "maxPerFile must be a whole number of at least 1",
        ),
        (
            json!({"pattern": "x", "limit": -3}),
            "limit must be a whole number of at least 1",
        ),
        (
            json!({"pattern": "x", "limit": "10"}),
            "limit must be a whole number of at least 1",
        ),
        (
            json!({"pattern": "x", "context": -1}),
            "context must be a whole number of at least 0",
        ),
        (
            json!({"pattern": "x", "path": ["a"]}),
            "path must be a string",
        ),
    ];
    for (args, expected) in cases {
        let error = refused(args.clone());
        assert!(error.contains(expected), "{args}: {error}");
    }
}
