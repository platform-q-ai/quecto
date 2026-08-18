//! Unit tests for `presentation_payloads` mappings.

use super::{is_subagent_note, recovered_thinking_page};

#[test]
fn recovered_thinking_page_projects_blocks_and_pagination() {
    let data = serde_json::json!({
        "thinking": [{"kind":"text","text":"first"}, {"kind":"redacted"}],
        "hasMoreThinking": true,
        "nextThinkingOffset": 42,
    });

    let page = recovered_thinking_page(&data);

    assert_eq!(page.blocks.len(), 2);
    assert_eq!(
        page.blocks[0].to_wire(),
        serde_json::json!({"kind":"text","text":"first"})
    );
    assert_eq!(
        page.blocks[1].to_wire(),
        serde_json::json!({"kind":"redacted"})
    );
    assert!(page.has_more);
    assert_eq!(page.next_offset, Some(42));
}

#[test]
fn recovered_thinking_page_defaults_absent_or_invalid_pagination() {
    let page = recovered_thinking_page(&serde_json::json!({
        "thinking": [{"kind":"text","text":"only"}],
        "hasMoreThinking": "yes",
        "nextThinkingOffset": "42",
    }));

    assert!(!page.has_more);
    assert_eq!(page.next_offset, None);
}

#[test]
fn detects_notes_verbatim_and_collapsed() {
    assert!(is_subagent_note(
        "<subagent_notification source=\"spawn_tool\" agent_id=\"poet\" sequence=\"1\">\nidle\n</subagent_notification>"
    ));
    // Ladder-collapsed form (context_pruning::message_collapse_stub).
    assert!(is_subagent_note(
        "[user: \"<subagent_notification source=\"spawn_tool\" agent_id=\"po\" (31 tokens) — recall(\"turn3:msg:user\")]"
    ));
    assert!(!is_subagent_note("write me a poem"));
    assert!(!is_subagent_note(
        "[user: \"write me a poem\" (4 tokens) — recall(\"turn1:msg:user\")]"
    ));
}

#[test]
fn subagents_parses_execution_backend_and_environment_from_snapshot() {
    // #1369 slice 4: the get_subagents response parse must round-trip the
    // additive versioned fields, not just live subagent_state_changed events.
    let data = serde_json::json!({
        "subagents": [{
            "agentId": "impl",
            "displayName": "impl",
            "agentUuid": "uuid-impl",
            "status": "running",
            "lastTool": null,
            "lastError": null,
            "pid": 7,
            "readOnly": false,
            "executionBackend": "script",
            "environment": {
                "ref": "C1",
                "name": "pr-env",
                "status": "running",
                "repository": "https://example.com/acme/widget.git",
                "branch": "pr-42",
                "runtimeId": "rt-9001",
                "workspace": "/work/pr-42",
                "socketMode": "proxy",
            },
        }],
    });
    let parsed = super::subagents(&data);
    assert_eq!(parsed.len(), 1);
    let info = &parsed[0];
    assert_eq!(info.execution_backend.as_deref(), Some("script"));
    let env = info.environment.as_ref().expect("environment parses");
    assert_eq!(env.environment_ref, "C1");
    assert_eq!(env.name.as_deref(), Some("pr-env"));
    assert_eq!(env.status, "running");
    assert_eq!(env.repository, "https://example.com/acme/widget.git");
    assert_eq!(env.branch.as_deref(), Some("pr-42"));
    assert_eq!(env.runtime_id, "rt-9001");
    assert_eq!(env.workspace, "/work/pr-42");
    assert_eq!(env.socket_mode, "proxy");
}
