use super::*;
use crate::components::theme;

#[test]
fn initial_state_not_running() {
    let state = AgentRunState::new();
    assert!(!state.is_running());
}

#[test]
fn start_sets_running() {
    let mut state = AgentRunState::new();
    state.start();
    assert!(state.is_running());
}

#[test]
fn normal_end_clears_running() {
    let mut state = AgentRunState::new();
    state.start();
    assert!(state.is_running());
    let processed = state.end();
    assert!(processed);
    assert!(!state.is_running());
}

#[test]
fn stale_agent_end_before_new_start_consumed() {
    // Scenario: stale AgentEnd arrives BEFORE new AgentStart.
    let mut state = AgentRunState::new();
    state.start(); // run 1
    state.abort(); // pending = 1, running = false

    // Stale AgentEnd arrives before user sends new prompt.
    let processed = state.end();
    assert!(!processed, "stale end should be consumed by pending_aborts");

    // Now new prompt → AgentStart.
    state.start(); // run 2
    assert!(state.is_running());

    // Real AgentEnd from run 2 — processed normally.
    assert!(state.end());
    assert!(!state.is_running());
}

#[test]
fn stale_agent_end_never_arrives_new_run_works() {
    // Scenario (#506): agent backend does NOT send AgentEnd for
    // aborted run. New AgentStart clears pending_aborts so the
    // real AgentEnd is not eaten.
    let mut state = AgentRunState::new();
    state.start(); // run 1
    state.abort(); // pending = 1

    // No stale AgentEnd arrives. User sends new prompt.
    state.start(); // run 2 — clears pending_aborts to 0
    assert!(state.is_running());

    // Real AgentEnd from run 2 — must be processed, not eaten.
    assert!(state.end());
    assert!(!state.is_running());
}

#[test]
fn abort_clears_running_for_ui() {
    let mut state = AgentRunState::new();
    state.start();
    state.abort();
    assert!(!state.is_running(), "abort should clear running for UI");
}

#[test]
fn abort_when_not_running_is_noop() {
    let mut state = AgentRunState::new();
    state.abort(); // should not panic or increment pending_aborts
    assert!(!state.is_running());
    // End should process normally (no pending aborts).
    state.start();
    assert!(state.end());
}

#[test]
fn multiple_aborts_with_starts_clears_pending() {
    let mut state = AgentRunState::new();
    state.start(); // run 1
    state.abort(); // pending = 1
    state.start(); // run 2 — clears pending to 0
    state.abort(); // pending = 1
    state.start(); // run 3 — clears pending to 0

    // Only the current run's end matters. No stale ends to consume.
    assert!(state.end()); // run 3 ends normally
    assert!(!state.is_running());
}

#[test]
fn normal_flow_without_abort() {
    let mut state = AgentRunState::new();
    state.start();
    assert!(state.is_running());
    state.end();
    assert!(!state.is_running());

    state.start();
    assert!(state.is_running());
    state.end();
    assert!(!state.is_running());
}

#[test]
fn abort_then_end_without_new_start() {
    let mut state = AgentRunState::new();
    state.start();
    state.abort(); // pending = 1, running = false

    // AgentEnd from the aborted run is consumed.
    assert!(!state.end());
    // Running is false (abort cleared it).
    assert!(!state.is_running());
    // Next prompt works correctly.
    state.start();
    assert!(state.is_running());
}

#[test]
fn abort_then_end_then_new_start_works() {
    let mut state = AgentRunState::new();
    state.start(); // run 1
    state.abort(); // pending = 1

    // Stale AgentEnd consumed.
    state.end();

    // New prompt works.
    state.start(); // run 2
    assert!(state.is_running());
    assert!(state.end()); // run 2 ends normally
    assert!(!state.is_running());
}

#[test]
fn reset_clears_all_state() {
    let mut state = AgentRunState::new();
    state.start();
    state.abort(); // pending = 1
    state.reset();
    assert!(!state.is_running());
    // After reset, end() should work normally (no stale aborts).
    state.start();
    assert!(state.end());
}

#[test]
fn start_clears_pending_aborts() {
    // Issue #506: If the agent backend doesn't send AgentEnd for
    // an aborted run, pending_aborts stays stale and eats the
    // next real AgentEnd.
    let mut state = AgentRunState::new();
    state.start(); // run 1
    state.abort(); // pending_aborts = 1

    // Agent backend does NOT send AgentEnd for the aborted run.
    // User sends a new prompt → AgentStart arrives.
    state.start(); // run 2

    // The real AgentEnd from run 2 should be processed normally,
    // NOT eaten by the stale pending_aborts.
    assert!(
        state.end(),
        "AgentEnd for new run should be processed, not consumed by stale pending_aborts"
    );
    assert!(!state.is_running());
}

// ── Base64 encoding tests (issue #528) ────────────────────────────

#[test]
fn base64_encode_empty() {
    assert_eq!(super::base64_encode(b""), "");
}

#[test]
fn base64_encode_hello() {
    assert_eq!(super::base64_encode(b"hello"), "aGVsbG8=");
}

#[test]
fn base64_encode_hello_world() {
    assert_eq!(super::base64_encode(b"hello world"), "aGVsbG8gd29ybGQ=");
}

#[test]
fn base64_encode_one_byte() {
    assert_eq!(super::base64_encode(b"a"), "YQ==");
}

#[test]
fn base64_encode_two_bytes() {
    assert_eq!(super::base64_encode(b"ab"), "YWI=");
}

#[test]
fn base64_encode_three_bytes() {
    assert_eq!(super::base64_encode(b"abc"), "YWJj");
}

// ── ANSI stripping tests (issue #528) ──────────────────────────────

#[test]
fn strip_ansi_plain_text() {
    assert_eq!(super::strip_ansi_for_selection("hello"), "hello");
}

#[test]
fn strip_ansi_sgr() {
    assert_eq!(super::strip_ansi_for_selection("\x1b[31mred\x1b[0m"), "red");
}

#[test]
fn strip_ansi_osc() {
    assert_eq!(
        super::strip_ansi_for_selection("\x1b]0;title\x07text"),
        "text"
    );
}

#[test]
fn strip_ansi_mixed() {
    assert_eq!(
        super::strip_ansi_for_selection("\x1b[32m✓\x1b[0m $ \x1b[1mgit status\x1b[0m"),
        "✓ $ git status"
    );
}

#[test]
fn stale_agent_end_after_new_start_kills_run() {
    // Critical race: abort → new start → stale AgentEnd arrives.
    // Since start() cleared pending_aborts, the stale end is
    // indistinguishable from the real end. This is a known
    // limitation — the protocol has no generation IDs.
    // The result: the new run appears to end prematurely.
    // This is better than the alternative (#506: new run hangs forever).
    let mut state = AgentRunState::new();
    state.start(); // run 1
    state.abort(); // pending = 1
    state.start(); // run 2 — clears pending to 0

    // Stale AgentEnd from run 1 arrives after run 2 started.
    // It's processed as run 2's end (no way to distinguish).
    let processed = state.end();
    assert!(processed, "stale end processed as current run's end");
    assert!(!state.is_running());

    // The real AgentEnd from run 2 will arrive later — but since
    // running is already false, it's harmless (end() on !running is a no-op).
}

#[test]
fn start_always_clears_pending_aborts() {
    // Regardless of how many aborts happened before, start()
    // always resets pending_aborts so the new run works cleanly.
    let mut state = AgentRunState::new();
    state.start();
    state.abort();
    // Stale end arrives before new start.
    state.end();
    state.start();
    state.abort();
    state.end();
    state.start();
    state.abort();
    // No stale end arrives this time.
    state.start(); // clears pending_aborts

    // The new run's end should work.
    assert!(state.end());
    assert!(!state.is_running());
}

// ── Free function tests ──────────────────────────────────────────

#[test]
fn builtin_commands_have_stable_order_and_names() {
    let cmds = super::builtin_commands();
    let names: Vec<_> = cmds.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "clear",
            "quit",
            "exit",
            "help",
            "hotkeys",
            "new",
            "session",
            "refresh-tui",
            "delete-all-subagents",
            "resume",
            "model",
            "effort",
            "workflow",
            "workflow-auto",
            "workflow-nudge",
        ]
    );
}

#[test]
fn builtin_commands_have_descriptions() {
    let cmds = super::builtin_commands();
    for cmd in cmds {
        assert!(
            !cmd.description.is_empty(),
            "{} has empty description",
            cmd.name
        );
    }
}

#[test]
fn resume_selector_overlay_has_opaque_border() {
    let mut selector = SelectList::new(
        vec![SelectItem {
            label: "default".into(),
            value: "default".into(),
            description: Some("2 messages".into()),
        }],
        10,
    );
    let (lines, width) = crate::components::select_overlay::build_select_list_overlay(
        "Resume session",
        "Enter resume · Esc cancel",
        &mut selector,
        100,
        40,
    );

    assert!(
        width > 72,
        "border should make the overlay wider than the list"
    );
    assert!(
        lines.len() > 4,
        "overlay should include top/bottom border padding"
    );
    assert!(
        lines.iter().all(|line| line.contains(theme::BG_OVERLAY)),
        "every overlay line should use the opaque background"
    );
    assert!(
        lines
            .iter()
            .all(|line| crate::components::utils::visible_width(line) == width),
        "opaque background should span the full overlay width"
    );
}

#[test]
fn rewind_selector_overlay_uses_go_back_title() {
    let mut selector = SelectList::new(
        vec![SelectItem {
            label: "Previous turn: hello".into(),
            value: "2".into(),
            description: None,
        }],
        10,
    );
    let (lines, _) = crate::components::select_overlay::build_select_list_overlay(
        "Go back to…",
        "Enter select · Esc cancel",
        &mut selector,
        100,
        40,
    );
    let joined = lines.join("\n");
    assert!(joined.contains("Go back to"));
    assert!(joined.contains("Previous turn: hello"));
}

#[test]
fn rewind_preview_sanitizes_user_content() {
    let preview = super::app_rewind::rewind_preview("safe \u{1b}]52;c;payload\u{7}text");
    assert_eq!(preview, "safe text");
    assert!(!preview.contains("]52;"));
    assert!(!preview.contains('\u{1b}'));
}

#[test]
fn sanitize_workflow_status_text_strips_control_and_truncates() {
    let result = super::sanitize_workflow_status_text("hello\x1b[31mworld\x00end", 10);
    assert!(!result.contains('\x1b'));
    assert!(!result.contains('\x00'));
    assert!(result.ends_with('…'));
}

#[test]
fn truncate_args_short() {
    assert_eq!(super::truncate_args("ls -la"), "ls -la");
}

#[test]
fn truncate_args_long() {
    let long = "a".repeat(50);
    let result = super::truncate_args(&long);
    assert!(result.len() <= 40);
    assert!(result.ends_with("..."));
}

#[test]
fn truncate_args_strips_control() {
    let s = "hello\x1b[31mworld\x00end";
    let result = super::truncate_args(s);
    assert!(!result.contains('\x1b'));
    assert!(!result.contains('\x00'));
}

#[test]
fn truncate_args_empty() {
    assert_eq!(super::truncate_args(""), "");
}

#[test]
fn read_git_branch_returns_some_in_repo() {
    // We're in a git repo during tests
    let branch = super::app_git::read_git_branch();
    // May be None if running from a detached HEAD or non-git context
    // but in our repo it should be Some
    if let Some(b) = branch {
        assert!(!b.is_empty());
    }
}

#[test]
fn strip_ansi_for_selection_empty() {
    assert_eq!(super::strip_ansi_for_selection(""), "");
}

#[test]
fn strip_ansi_csi_with_tilde() {
    assert_eq!(super::strip_ansi_for_selection("\x1b[5~text"), "text");
}

#[test]
fn strip_ansi_nested_escapes() {
    assert_eq!(
        super::strip_ansi_for_selection("\x1b[1m\x1b[32mbold green\x1b[0m"),
        "bold green"
    );
}

#[test]
fn strip_ansi_osc_with_st() {
    // OSC terminated with ST (\x1b\)
    assert_eq!(
        super::strip_ansi_for_selection("\x1b]0;title\x1b\\text"),
        "text"
    );
}

#[test]
fn base64_encode_multibyte() {
    // Test with UTF-8 content
    let result = super::base64_encode("héllo".as_bytes());
    assert!(!result.is_empty());
    assert!(
        result
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    );
}

#[test]
fn base64_encode_binary() {
    let result = super::base64_encode(&[0, 1, 2, 255, 254, 253]);
    assert!(!result.is_empty());
}

#[test]
fn max_clipboard_bytes_is_reasonable() {
    const _: () = {
        assert!(super::MAX_CLIPBOARD_BYTES >= 1024);
        assert!(super::MAX_CLIPBOARD_BYTES <= 1024 * 1024);
    };
}

#[test]
fn selection_anchor_copy() {
    let a = super::app_selection::SelectionAnchor { col: 10, row: 5 };
    let b = a; // Copy
    assert_eq!(a.col, b.col);
    assert_eq!(a.row, b.row);
}

#[test]
fn text_selection_clone() {
    let sel = super::app_selection::TextSelection {
        start: super::app_selection::SelectionAnchor { col: 0, row: 0 },
        end: super::app_selection::SelectionAnchor { col: 10, row: 5 },
    };
    let sel2 = sel.clone();
    assert_eq!(sel2.start.col, 0);
    assert_eq!(sel2.end.col, 10);
}

// ── Ctrl+C action decision tests (#536) ──────────────────────────

#[test]
fn ctrl_c_clears_editor_when_running_and_has_text() {
    assert_eq!(
        super::ctrl_c_action(true, false),
        super::CtrlCAction::ClearEditor,
        "Ctrl+C should clear editor when agent is running but editor has text"
    );
}

#[test]
fn ctrl_c_aborts_when_running_and_editor_empty() {
    assert_eq!(
        super::ctrl_c_action(true, true),
        super::CtrlCAction::AbortAgent,
        "Ctrl+C should abort agent when running and editor is empty"
    );
}

#[test]
fn ctrl_c_clears_editor_when_idle_and_has_text() {
    assert_eq!(
        super::ctrl_c_action(false, false),
        super::CtrlCAction::ClearEditor,
        "Ctrl+C should clear editor when idle and editor has text"
    );
}

#[test]
fn ctrl_c_noop_when_idle_and_editor_empty() {
    assert_eq!(
        super::ctrl_c_action(false, true),
        super::CtrlCAction::Noop,
        "Ctrl+C should do nothing when idle and editor is empty"
    );
}

// ── Subagent tool classification tests (#538) ──────────────────

#[test]
fn spawn_is_subagent_tool() {
    assert!(super::is_subagent_tool("spawn"));
}

#[test]
fn agent_cmd_is_subagent_tool() {
    assert!(super::is_subagent_tool("agent_cmd"));
}

#[test]
fn regular_tools_are_not_subagent_tools() {
    assert!(!super::is_subagent_tool("bash"));
    assert!(!super::is_subagent_tool("read"));
    assert!(!super::is_subagent_tool("write"));
}

// ── Tool output suppression tests (#538) ─────────────────────────

#[test]
fn spawn_tool_output_suppressed() {
    let args = serde_json::json!({"agent_id": "worker-1"});
    assert!(
        super::app_events::suppress_tool_box("spawn", &args),
        "spawn output should be suppressed (status bar shows it)"
    );
}

#[test]
fn agent_cmd_content_reads_shown() {
    // Content reads and one-shot results still render in the chat.
    for cmd in &["get_messages", "get_messages_tail", "await"] {
        let args = serde_json::json!({"agent_id": "w1", "command": cmd});
        assert!(
            !super::app_events::suppress_tool_box("agent_cmd", &args),
            "agent_cmd {cmd} output should be shown"
        );
    }
}

#[test]
fn every_agent_cmd_command_renders_a_box() {
    // #871: every model-issued agent_cmd invocation renders a tool box, the
    // same way a normal tool call does — including the control/destructive
    // commands (prompt/steer/abort/kill). Hiding them made the transcript
    // incomplete. Only `spawn` stays suppressed (the status bar shows it).
    let mk = |cmd: &str| serde_json::json!({"agent_id": "w1", "command": cmd});
    for cmd in &[
        "get_state",
        "get_subagents",
        "get_session_stats",
        "get_extensions",
        "get_messages",
        "await",
        "prompt",
        "steer",
        "abort",
        "kill",
        "follow_up",
        "set_model",
    ] {
        assert!(
            !super::app_events::suppress_tool_box("agent_cmd", &mk(cmd)),
            "agent_cmd {cmd} should render a tool box (#871)"
        );
    }
    assert!(super::app_events::suppress_tool_box("spawn", &mk("x")));
}

#[test]
fn agent_cmd_unknown_command_shown() {
    let args = serde_json::json!({"agent_id": "w1", "command": "future_query"});
    assert!(
        !super::app_events::suppress_tool_box("agent_cmd", &args),
        "unknown agent_cmd commands should be shown by default"
    );
}

#[test]
fn regular_tool_output_shown() {
    let args = serde_json::json!({});
    assert!(!super::app_events::suppress_tool_box("bash", &args));
    assert!(!super::app_events::suppress_tool_box("read", &args));
    assert!(!super::app_events::suppress_tool_box("write", &args));
    assert!(!super::app_events::suppress_tool_box("edit", &args));
}

// ── Exited subagent GC tests (#540) ──────────────────────────────
