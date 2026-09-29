//! The stream vocabulary's own rules (#2285).

use serde_json::json;

use super::*;

fn init(statuses: &[(&str, &str)], tools: &[&str]) -> InitEvent {
    InitEvent {
        tools: tools.iter().map(|t| t.to_string()).collect(),
        mcp_servers: statuses
            .iter()
            .map(|(name, status)| McpServerStatus {
                name: name.to_string(),
                status: status.to_string(),
            })
            .collect(),
        ..InitEvent::default()
    }
}

// Mapping row: `system/init` → launch check that
// `mcp_servers[].status == "connected"` and the tools list is exact.
#[test]
fn the_launch_check_needs_every_mcp_server_connected() {
    assert!(init(&[("board", "connected")], &[]).mcp_servers_connected());
    assert!(init(&[], &[]).mcp_servers_connected());
    assert!(!init(&[("board", "connected"), ("quecto", "failed")], &[]).mcp_servers_connected());
    assert!(!init(&[("board", "pending")], &[]).mcp_servers_connected());
}

#[test]
fn the_launch_check_needs_the_exact_tool_list() {
    let session = init(&[], &["Bash", "Read", "mcp__board__board_claim"]);
    assert!(session.tools_are_exactly(&["mcp__board__board_claim", "Read", "Bash"]));
    assert!(!session.tools_are_exactly(&["Bash", "Read"]));
    assert!(!session.tools_are_exactly(&["Bash", "Read", "mcp__board__board_claim", "WebFetch"]));
}

fn tool_result(content: serde_json::Value) -> ToolResultEvent {
    ToolResultEvent {
        tool_use_id: Some("toolu_1".into()),
        content,
        is_error: false,
        permission_denied: false,
    }
}

#[test]
fn tool_result_text_reads_a_string_or_text_blocks() {
    assert_eq!(
        tool_result(json!("Hello, World!")).content_text(),
        "Hello, World!"
    );
    assert_eq!(
        tool_result(json!([
            {"type": "text", "text": "{\"sent\": "},
            {"type": "image", "source": {}},
            {"type": "text", "text": "true}"}
        ]))
        .content_text(),
        "{\"sent\": \ntrue}"
    );
    assert_eq!(tool_result(json!(null)).content_text(), "");
}

// Mapping row: `rate_limit_event.rate_limit_info` (`status`).
#[test]
fn only_an_allowed_rate_limit_status_needs_no_warning() {
    assert_eq!(RateLimitStatus::parse("allowed"), RateLimitStatus::Allowed);
    assert!(!RateLimitStatus::parse("allowed").warrants_warning());
    assert!(RateLimitStatus::parse("allowed_warning").warrants_warning());
    assert!(RateLimitStatus::parse("rejected").warrants_warning());
    assert_eq!(
        RateLimitStatus::parse("throttled"),
        RateLimitStatus::Other("throttled".into())
    );
    assert!(RateLimitStatus::parse("throttled").warrants_warning());
}

fn versioned(version: Option<&str>, capabilities: &[&str]) -> InitEvent {
    InitEvent {
        cli_version: version.map(str::to_string),
        capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
        ..InitEvent::default()
    }
}

// #2287 review round 3 (L1): claude advertises no capability for naming
// the user turns a result consumed; a CLI at or past the version verified
// to (2.1.280) does, any older or unreadable version is not known to.
#[test]
fn a_cli_names_turns_from_the_verified_version_on() {
    assert_eq!(NAMES_TURNS_SINCE, [2, 1, 280]);
    for named in ["2.1.280", "2.1.281", "2.2.0", "3.0.0", "2.1.1000"] {
        assert!(versioned(Some(named), &[]).names_turns(), "{named}");
    }
    for unknown in [
        "2.1.279", "2.0.999", "1.9.300", "2.1", "2", "", "v2.1.280", "x.y.z",
    ] {
        assert!(!versioned(Some(unknown), &[]).names_turns(), "{unknown}");
    }
    assert!(!versioned(None, &[]).names_turns());
}

// #2287 review round 4 (N1): only a plain `major.minor.patch` release is
// read. A pre-release (`2.1.280-beta.1` precedes 2.1.280, which was the
// version verified) or any other suffix is not known to name turns: it
// fails safe, and steers wait for a result that names them.
#[test]
fn a_pre_release_or_suffixed_version_is_not_known_to_name_turns() {
    for suffixed in [
        "2.1.280-beta.1",
        "2.1.281-rc.1",
        "3.0.0-alpha",
        "2.1.280+build.5",
        "2.1.280.1",
        "2.1.280 ",
        "2.1.280x",
    ] {
        assert!(!versioned(Some(suffixed), &[]).names_turns(), "{suffixed}");
    }
}

// #2287 review round 3 (L3): only a CLI advertising
// `interrupt_cancel_queued_v1` withdraws the queued user turns on an
// interrupt.
#[test]
fn a_cli_cancels_queued_turns_only_when_it_advertises_it() {
    assert_eq!(CANCEL_QUEUED_CAPABILITY, "interrupt_cancel_queued_v1");
    assert!(
        versioned(
            None,
            &["interrupt_receipt_v1", "interrupt_cancel_queued_v1"]
        )
        .cancels_queued()
    );
    assert!(!versioned(Some("2.1.280"), &["interrupt_receipt_v1"]).cancels_queued());
    assert!(!versioned(Some("2.1.280"), &[]).cancels_queued());
}
