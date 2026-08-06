//! Slice 3 (#1369): typed parent endpoint parsing. The create/exec result
//! contracts accept EXACTLY ONE of `socket_path` or a validated `socket_proxy`
//! argv; the parsed endpoint is a typed value, never reconstructed from a
//! requested path.

use super::*;
use crate::domain::subagent_launch::ParentEndpoint;

fn create_json(endpoint_fields: &str) -> Vec<u8> {
    format!(
        r#"{{"environment_id":"env-1","workspace_path":"/tmp/ws","metadata":{{}},{endpoint_fields}}}"#
    )
    .into_bytes()
}

#[test]
fn create_result_accepts_direct_endpoint_only() {
    let result = parse_create_result(&create_json(r#""socket_path":"/tmp/x.sock""#)).unwrap();
    assert_eq!(
        result.endpoint,
        ParentEndpoint::Direct {
            socket_path: std::path::PathBuf::from("/tmp/x.sock")
        }
    );
}

#[test]
fn create_result_accepts_proxy_endpoint_only() {
    let result =
        parse_create_result(&create_json(r#""socket_proxy":{"argv":["proxy","arg"]}"#)).unwrap();
    assert_eq!(
        result.endpoint,
        ParentEndpoint::Proxy {
            argv: vec!["proxy".to_string(), "arg".to_string()]
        }
    );
}

#[test]
fn create_result_rejects_both_endpoints() {
    let err = parse_create_result(&create_json(
        r#""socket_path":"/tmp/x.sock","socket_proxy":{"argv":["proxy"]}"#,
    ))
    .unwrap_err();
    assert!(err.to_string().contains("exactly one"), "{err}");
}

#[test]
fn create_result_rejects_empty_socket_path_alongside_proxy() {
    // A present-but-empty socket_path is still a present endpoint field: a
    // direct-mode template buggily also carrying socket_proxy must fail the
    // exactly-one check, never silently collapse into proxy mode.
    let err = parse_create_result(&create_json(
        r#""socket_path":"","socket_proxy":{"argv":["proxy"]}"#,
    ))
    .unwrap_err();
    assert!(err.to_string().contains("exactly one"), "{err}");
}

#[test]
fn create_result_rejects_empty_socket_path_alone() {
    let err = parse_create_result(&create_json(r#""socket_path":"""#)).unwrap_err();
    assert!(err.to_string().contains("non-empty"), "{err}");
}

#[test]
fn create_result_rejects_missing_endpoint() {
    // An unknown key is rejected by the strict wire contract, naming the
    // offending field.
    let err = parse_create_result(&create_json(r#""metadata_extra":null"#)).unwrap_err();
    assert!(err.to_string().contains("metadata_extra"), "{err}");
    // A result with NEITHER endpoint field fails the exactly-one requirement.
    let err2 = parse_create_result(
        br#"{"environment_id":"env-1","workspace_path":"/tmp/ws","metadata":{}}"#,
    )
    .unwrap_err();
    assert!(err2.to_string().contains("exactly one"), "{err2}");
}

#[test]
fn create_result_rejects_empty_proxy_argv() {
    let err = parse_create_result(&create_json(r#""socket_proxy":{"argv":[]}"#)).unwrap_err();
    assert!(err.to_string().contains("socket_proxy"), "{err}");
}

#[test]
fn create_result_rejects_unsafe_proxy_argv() {
    for argv in [r#"["", "x"]"#, "[\"a\\u0000b\"]"] {
        let err = parse_create_result(&create_json(&format!(
            r#""socket_proxy":{{"argv":{argv}}}"#
        )))
        .unwrap_err();
        assert!(err.to_string().contains("socket_proxy"), "{err}");
    }
}

#[test]
fn create_result_rejects_unknown_proxy_fields() {
    let err = parse_create_result(&create_json(
        r#""socket_proxy":{"argv":["proxy"],"shell":"sh -c"}"#,
    ))
    .unwrap_err();
    // The rejection must name the unknown proxy field, not be an arbitrary
    // failure (review: a vacuous non-empty check cannot distinguish causes).
    assert!(
        err.to_string().contains("shell") || err.to_string().contains("unknown"),
        "{err}"
    );
}

#[test]
fn exec_result_accepts_proxy_endpoint_only() {
    let endpoint =
        parse_exec_result(br#"{"metadata":{},"socket_proxy":{"argv":["proxy","join"]}}"#).unwrap();
    assert_eq!(
        endpoint,
        ParentEndpoint::Proxy {
            argv: vec!["proxy".to_string(), "join".to_string()]
        }
    );
}

#[test]
fn exec_result_accepts_direct_endpoint_only() {
    let endpoint = parse_exec_result(br#"{"metadata":{},"socket_path":"/tmp/y.sock"}"#).unwrap();
    assert_eq!(
        endpoint,
        ParentEndpoint::Direct {
            socket_path: std::path::PathBuf::from("/tmp/y.sock")
        }
    );
}

#[test]
fn exec_result_rejects_both_endpoints() {
    let err = parse_exec_result(
        br#"{"metadata":{},"socket_path":"/tmp/y.sock","socket_proxy":{"argv":["proxy"]}}"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("exactly one"), "{err}");
}

#[test]
fn exec_result_rejects_missing_endpoint() {
    let err = parse_exec_result(br#"{"metadata":{}}"#).unwrap_err();
    assert!(err.to_string().contains("exactly one"), "{err}");
}
