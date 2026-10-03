use std::time::Duration;

use super::*;

fn registration() -> QuectoToolRegistration {
    QuectoToolRegistration {
        name: "browser_screenshot".into(),
        description: "Screenshot".into(),
        parameters_schema: r#"{"type":"object"}"#.into(),
        timeout_seconds: None,
    }
}

/// #2423 review: each tool is registered with a timeout that outlasts the
/// MCP HTTP timeout (`--timeout`) by 5 s, within Quecto's 1 to 600.
#[test]
fn every_tool_waits_as_long_as_an_mcp_call_may_take() {
    let cases = [
        (Duration::from_secs(30), 35),
        (Duration::from_secs(0), 5),
        (Duration::from_millis(1500), 7),
        (Duration::from_secs(595), 600),
        (Duration::from_secs(3600), 600),
        (Duration::MAX, 600),
    ];
    for (mcp_timeout, expected) in cases {
        let registered = with_tool_timeout(vec![registration(), registration()], mcp_timeout);
        let seconds: Vec<Option<u64>> = registered.iter().map(|r| r.timeout_seconds).collect();
        assert_eq!(seconds, [Some(expected); 2], "{mcp_timeout:?}");
    }
}

#[test]
fn the_timeout_is_sent_as_timeout_seconds_only_when_set() {
    let set = with_tool_timeout(vec![registration()], Duration::from_secs(60));
    let value = serde_json::to_value(&set[0]).unwrap();
    assert_eq!(value["timeoutSeconds"], 65);
    let unset = serde_json::to_value(registration()).unwrap();
    assert!(unset.get("timeoutSeconds").is_none());
}
