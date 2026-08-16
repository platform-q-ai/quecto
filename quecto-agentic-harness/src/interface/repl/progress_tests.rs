// Unit tests for the REPL progress rendering subsystem.
//
// These tests verify:
// - ProgressRenderer is silent when is_tty = false (no stderr writes)
// - ProgressRenderer handles all event types without panicking
// - Spinner tick advances frames
// - ANSI erase sequences are used (contains "\r" and/or "\x1b")
// - ProgressChannel wires events correctly

use super::*;
use crate::domain::agent::AgentProgressEvent;
use std::sync::{Arc, Mutex};

fn sample_thinking_event() -> AgentProgressEvent {
    AgentProgressEvent::Thinking {
        context_tokens: 0,
        max_context_tokens: 100,
        provider: "openai".to_string(),
        model: "gpt-5.4".to_string(),
    }
}

// ---------------------------------------------------------------------------
// ProgressRenderer: non-TTY mode
// ---------------------------------------------------------------------------

#[test]
fn test_progress_renderer_non_tty_produces_no_output() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = ProgressRenderer::new_with_writer(false, buf.clone());

    renderer.handle_event(sample_thinking_event());
    renderer.handle_event(AgentProgressEvent::ToolStarted {
        tool_call_id: String::new(),
        name: "bash".to_string(),
        arguments: "echo hi".to_string(),
    });
    renderer.handle_event(AgentProgressEvent::ToolFinished {
        tool_call_id: String::new(),
        name: "bash".to_string(),
        arguments: "{\"command\": \"echo hi\"}".to_string(),
        result_content: String::new(),
        duration_ms: 42,
        is_error: false,
    });
    renderer.handle_event(AgentProgressEvent::Done);

    let output = buf.lock().unwrap();
    assert!(
        output.is_empty(),
        "non-TTY renderer should not write anything, got: {:?}",
        String::from_utf8_lossy(&output)
    );
}

// ---------------------------------------------------------------------------
// ProgressRenderer: TTY mode — output content
// ---------------------------------------------------------------------------

#[test]
fn test_progress_renderer_tty_thinking_writes_output() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = ProgressRenderer::new_with_writer(true, buf.clone());

    renderer.handle_event(sample_thinking_event());

    let output = buf.lock().unwrap();
    let text = String::from_utf8_lossy(&output);
    assert!(
        !text.is_empty(),
        "TTY renderer should write something for Thinking event"
    );
    // Should include a carriage return for in-place update
    assert!(
        text.contains('\r'),
        "expected carriage return in output, got: {:?}",
        text
    );
    // Should mention "Thinking"
    assert!(
        text.to_lowercase().contains("thinking"),
        "expected 'thinking' in output, got: {:?}",
        text
    );
}

#[test]
fn test_progress_renderer_tty_tool_started_shows_tool_name() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = ProgressRenderer::new_with_writer(true, buf.clone());

    renderer.handle_event(AgentProgressEvent::ToolStarted {
        tool_call_id: String::new(),
        name: "read".to_string(),
        arguments: "src/main.rs".to_string(),
    });

    let output = buf.lock().unwrap();
    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains("read"),
        "expected tool name 'read' in output, got: {:?}",
        text
    );
}

#[test]
fn test_progress_renderer_tty_tool_started_shows_tool_arguments() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = ProgressRenderer::new_with_writer(true, buf.clone());

    renderer.handle_event(AgentProgressEvent::ToolStarted {
        tool_call_id: String::new(),
        name: "bash".to_string(),
        arguments: "{\"command\": \"echo hi\"}".to_string(),
    });

    let output = buf.lock().unwrap();
    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains("echo hi"),
        "expected tool arguments in output, got: {:?}",
        text
    );
}

#[test]
fn test_progress_renderer_tty_tool_finished_shows_tool_name_and_duration() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = ProgressRenderer::new_with_writer(true, buf.clone());

    renderer.handle_event(AgentProgressEvent::ToolFinished {
        tool_call_id: String::new(),
        name: "bash".to_string(),
        arguments: "{\"command\": \"echo hi\"}".to_string(),
        result_content: String::new(),
        duration_ms: 1234,
        is_error: false,
    });

    let output = buf.lock().unwrap();
    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains("bash"),
        "expected 'bash' in finished output, got: {:?}",
        text
    );
    assert!(
        text.contains("echo hi"),
        "expected tool arguments in finished output, got: {:?}",
        text
    );
    assert!(
        text.contains("1234"),
        "expected duration '1234' in finished output, got: {:?}",
        text
    );
}

#[test]
fn test_progress_renderer_tty_tool_finished_error_indicates_failure() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = ProgressRenderer::new_with_writer(true, buf.clone());

    renderer.handle_event(AgentProgressEvent::ToolFinished {
        tool_call_id: String::new(),
        name: "bash".to_string(),
        arguments: "{\"command\": \"echo fail\"}".to_string(),
        result_content: String::new(),
        duration_ms: 50,
        is_error: true,
    });

    let output = buf.lock().unwrap();
    let text = String::from_utf8_lossy(&output);
    // Should signal failure somehow (✗ or "error" or "err" or "!")
    let lower = text.to_lowercase();
    assert!(
        lower.contains("err") || text.contains('✗') || text.contains('!') || text.contains('✕'),
        "expected error indicator in finished-error output, got: {:?}",
        text
    );
}

#[test]
fn test_progress_renderer_tty_done_clears_line() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = ProgressRenderer::new_with_writer(true, buf.clone());

    // Simulate a thinking event first, then done
    renderer.handle_event(sample_thinking_event());
    {
        let mut locked = buf.lock().unwrap();
        locked.clear(); // reset buffer to check only the Done output
    }

    renderer.handle_event(AgentProgressEvent::Done);

    let output = buf.lock().unwrap();
    let text = String::from_utf8_lossy(&output);
    // Done should erase the current line (carriage return + erase or blank line)
    assert!(
        text.contains('\r') || text.contains('\n'),
        "expected line-clearing in Done output, got: {:?}",
        text
    );
}

// ---------------------------------------------------------------------------
// Spinner frame advancement
// ---------------------------------------------------------------------------

#[test]
fn test_spinner_frames_are_non_empty() {
    for frame in SPINNER_FRAMES.iter() {
        let f: &str = frame;
        assert!(!f.is_empty(), "spinner frame should not be empty");
    }
}

#[test]
fn test_spinner_frame_count_is_reasonable() {
    // Standard braille spinner has 10 frames
    assert!(
        SPINNER_FRAMES.len() >= 4,
        "expected at least 4 spinner frames"
    );
}

#[test]
fn test_progress_renderer_tick_advances_frame() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = ProgressRenderer::new_with_writer(true, buf.clone());

    // First set an active status line so there's something to redraw
    renderer.handle_event(sample_thinking_event());
    {
        let mut locked = buf.lock().unwrap();
        locked.clear(); // reset to measure only tick output
    }

    // Tick several times — should not panic and should write something each tick
    for _ in 0..SPINNER_FRAMES.len() + 2 {
        renderer.tick();
    }

    let output = buf.lock().unwrap();
    assert!(
        !output.is_empty(),
        "tick() on TTY renderer should write spinner output after status is set"
    );
}

#[test]
fn test_progress_renderer_tick_non_tty_no_output() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = ProgressRenderer::new_with_writer(false, buf.clone());

    for _ in 0..5 {
        renderer.tick();
    }

    let output = buf.lock().unwrap();
    assert!(
        output.is_empty(),
        "tick() on non-TTY renderer should not write anything"
    );
}

// ---------------------------------------------------------------------------
// ProgressChannel: event wiring
// ---------------------------------------------------------------------------

#[test]
fn test_progress_channel_sends_events() {
    let (tx, rx) = std::sync::mpsc::channel::<AgentProgressEvent>();
    let callback = make_channel_callback(tx);

    callback(sample_thinking_event());
    callback(AgentProgressEvent::ToolStarted {
        tool_call_id: String::new(),
        name: "bash".to_string(),
        arguments: "echo hi".to_string(),
    });
    callback(AgentProgressEvent::Done);

    let events: Vec<AgentProgressEvent> = rx.try_iter().collect();
    assert_eq!(events.len(), 3, "expected 3 events, got {:?}", events.len());
    assert!(
        matches!(events[0], AgentProgressEvent::Thinking { .. }),
        "first event should be Thinking"
    );
    assert!(
        matches!(events[1], AgentProgressEvent::ToolStarted { .. }),
        "second event should be ToolStarted"
    );
    assert!(
        matches!(events[2], AgentProgressEvent::Done),
        "third event should be Done"
    );
}

#[test]
fn test_progress_channel_send_after_receiver_dropped_does_not_panic() {
    let (tx, rx) = std::sync::mpsc::channel::<AgentProgressEvent>();
    let callback = make_channel_callback(tx);

    // Drop receiver — sends should silently fail, not panic
    drop(rx);
    callback(sample_thinking_event());
    callback(AgentProgressEvent::Done);
    // Should reach here without panicking
}

// ---------------------------------------------------------------------------
// sanitize_for_terminal
// ---------------------------------------------------------------------------

#[test]
fn test_sanitize_strips_ansi_escape() {
    // ESC sequence: clear screen
    let input = "bash\x1b[2Jclean";
    let result = sanitize_for_terminal(input);
    assert!(
        !result.contains('\x1b'),
        "expected ESC to be stripped, got: {:?}",
        result
    );
    assert!(
        result.contains("bash"),
        "expected 'bash' to remain, got: {:?}",
        result
    );
}

#[test]
fn test_sanitize_strips_carriage_return() {
    let input = "bash\rmalicious";
    let result = sanitize_for_terminal(input);
    assert!(
        !result.contains('\r'),
        "expected \\r to be stripped, got: {:?}",
        result
    );
}

#[test]
fn test_sanitize_strips_null_byte() {
    let input = "bash\x00evil";
    let result = sanitize_for_terminal(input);
    assert!(
        !result.contains('\x00'),
        "expected null byte to be stripped, got: {:?}",
        result
    );
}

#[test]
fn test_sanitize_passes_normal_tool_names() {
    let names = ["bash", "read", "write", "web_search", "recall"];
    for name in &names {
        let result = sanitize_for_terminal(name);
        assert_eq!(&result, name, "expected normal name to pass unchanged");
    }
}

#[test]
fn test_sanitize_passes_unicode() {
    // Non-ASCII Unicode (e.g. emoji in tool names) should pass through
    let input = "tool_✓";
    let result = sanitize_for_terminal(input);
    assert_eq!(result, input, "expected unicode to pass through");
}

#[test]
fn test_sanitize_strips_osc_sequence() {
    // OSC 52 clipboard injection attempt
    let input = "bash\x1b]52;c;dGVzdA==\x07real";
    let result = sanitize_for_terminal(input);
    assert!(
        !result.contains('\x1b'),
        "expected ESC to be stripped from OSC, got: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// AgentProgressEvent: Debug / Clone
// ---------------------------------------------------------------------------

#[test]
fn test_agent_progress_event_debug_and_clone() {
    let events = vec![
        sample_thinking_event(),
        AgentProgressEvent::ToolStarted {
            tool_call_id: String::new(),
            name: "bash".to_string(),
            arguments: "echo hi".to_string(),
        },
        AgentProgressEvent::ToolFinished {
            tool_call_id: String::new(),
            name: "bash".to_string(),
            arguments: "{\"command\": \"echo hi\"}".to_string(),
            result_content: String::new(),
            duration_ms: 100,
            is_error: false,
        },
        AgentProgressEvent::Done,
    ];

    for event in &events {
        let cloned = event.clone();
        let debug_str = format!("{:?}", cloned);
        assert!(!debug_str.is_empty());
    }
}

// --- Helper function unit tests ---

#[test]
fn test_sanitize_for_terminal_strips_ansi() {
    assert_eq!(sanitize_for_terminal("\x1b[31mred\x1b[0m"), "[31mred[0m");
}

#[test]
fn test_sanitize_for_terminal_strips_control() {
    assert_eq!(sanitize_for_terminal("a\x00b\x01c"), "abc");
}

#[test]
fn test_sanitize_for_terminal_preserves_unicode() {
    assert_eq!(sanitize_for_terminal("héllo 🦀"), "héllo 🦀");
}

#[test]
fn test_sanitize_and_truncate_short() {
    assert_eq!(sanitize_and_truncate("hello", 10), "hello");
}

#[test]
fn test_sanitize_and_truncate_long() {
    let long = "a".repeat(200);
    let result = sanitize_and_truncate(&long, 10);
    assert!(result.len() <= 13); // 10 chars + possible "..."
}

#[test]
fn test_format_compact_tokens_small() {
    assert_eq!(format_compact_tokens(500), "500");
}

#[test]
fn test_format_compact_tokens_thousands() {
    assert_eq!(format_compact_tokens(1000), "1k");
    assert_eq!(format_compact_tokens(1500), "1.5k");
    assert_eq!(format_compact_tokens(2000), "2k");
    assert_eq!(format_compact_tokens(15000), "15k");
}

#[test]
fn test_format_context_usage() {
    assert_eq!(format_context_usage(10000, 200000), "5.0%/200k");
}

#[test]
fn test_format_context_usage_zero_max() {
    assert_eq!(format_context_usage(0, 0), "0.0%/0");
}

#[test]
fn test_format_status_detail() {
    let result = format_status_detail(10000, 200000, "anthropic", "claude-4");
    assert!(result.contains("5.0%/200k"));
    assert!(result.contains("anthropic"));
    assert!(result.contains("claude-4"));
}

#[test]
fn test_format_tool_status_with_args() {
    let result = format_tool_status("bash", r#"{"command": "ls -la"}"#);
    assert!(result.contains("bash"));
    assert!(result.contains("ls -la"));
}

#[test]
fn test_format_tool_status_empty_args() {
    let result = format_tool_status("bash", "");
    assert_eq!(result, "bash");
}

#[test]
fn test_format_tool_status_sanitizes() {
    let result = format_tool_status("bash\x1b[31m", "echo\x00hi");
    assert!(!result.contains('\x1b'));
    assert!(!result.contains('\x00'));
}

#[test]
fn test_spinner_frames_not_empty() {
    assert!(!SPINNER_FRAMES.is_empty());
}

#[test]
fn test_spinner_frames_all_non_empty() {
    for frame in SPINNER_FRAMES {
        assert!(!frame.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Status header / detail rendering + multi-line clear
// ---------------------------------------------------------------------------

#[test]
fn test_new_tty_capture_with_status_renders_header_and_clears() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer =
        ProgressRenderer::new_tty_capture_with_status(buf.clone(), Some("/work (main)".into()));

    // Thinking sets the status detail too, so render emits header + detail
    // lines beneath the spinner (current_line_count > 0).
    renderer.handle_event(sample_thinking_event());
    let after_render = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
    assert!(
        after_render.contains("/work (main)"),
        "status header should render, got: {:?}",
        after_render
    );

    // Done must clear the spinner line and the status lines beneath it,
    // exercising the multi-line clear (cursor-up) branch.
    buf.lock().unwrap().clear();
    renderer.handle_event(AgentProgressEvent::Done);
    let after_done = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
    assert!(after_done.contains('\r'));
    assert!(
        after_done.contains("\x1b["),
        "multi-line clear should emit a cursor-up escape, got: {:?}",
        after_done
    );
}

#[test]
fn test_new_with_status_none_header_still_renders_tool() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = ProgressRenderer::new_with_status(true, MutexVecWriter(buf.clone()), None);
    renderer.handle_event(AgentProgressEvent::ToolStarted {
        tool_call_id: String::new(),
        name: "read".into(),
        arguments: String::new(),
    });
    assert!(!buf.lock().unwrap().is_empty());
}

#[test]
fn test_new_with_status_empty_header_is_skipped() {
    // An empty header string sanitizes to "" and is filtered out of the
    // status lines (no spinner-adjacent header rendered).
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer =
        ProgressRenderer::new_with_status(true, MutexVecWriter(buf.clone()), Some(String::new()));
    renderer.handle_event(AgentProgressEvent::ToolStarted {
        tool_call_id: String::new(),
        name: "read".into(),
        arguments: "x".into(),
    });
    let rendered = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
    assert!(!rendered.is_empty());
    assert!(
        !rendered.contains("\n\x1b["),
        "empty header should not render status lines, got: {:?}",
        rendered
    );
}

#[test]
fn test_mutex_vec_writer_write_is_covered_for_function_threshold_stability() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut writer = MutexVecWriter(buf.clone());
    writer.write_all(b"covered").unwrap();
    writer.flush().unwrap();
    assert_eq!(buf.lock().unwrap().as_slice(), b"covered");
}

// ---------------------------------------------------------------------------
// Token event (no-op spinner path)
// ---------------------------------------------------------------------------

#[test]
fn test_handle_token_event_is_noop() {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = ProgressRenderer::new_with_writer(true, buf.clone());
    renderer.handle_event(AgentProgressEvent::Token("hello".into()));
    assert!(
        buf.lock().unwrap().is_empty(),
        "Token events are forwarded elsewhere; spinner writes nothing"
    );
}

// ---------------------------------------------------------------------------
// read_git_branch / build_status_header_line
// ---------------------------------------------------------------------------

#[test]
fn test_read_git_branch_parses_symbolic_ref() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(
        dir.path().join(".git").join("HEAD"),
        "ref: refs/heads/main\n",
    )
    .unwrap();
    assert_eq!(read_git_branch(dir.path()), Some("main".to_string()));
}

#[test]
fn test_read_git_branch_detached_head_is_none() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join(".git").join("HEAD"), "0123456789abcdef\n").unwrap();
    assert_eq!(read_git_branch(dir.path()), None);
}

#[test]
fn test_read_git_branch_missing_head_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read_git_branch(dir.path()), None);
}

#[test]
fn test_build_status_header_line_returns_nonempty() {
    // The current working directory always exists, so this yields Some(path).
    let line = build_status_header_line();
    let line = line.expect("cwd should produce a header line");
    assert!(!line.is_empty());
}

// ---------------------------------------------------------------------------
// spawn_spinner_thread_with_status: stop + drop lifecycles
// ---------------------------------------------------------------------------

#[test]
fn test_spawn_spinner_thread_stop_joins_cleanly() {
    let (callback, handle) = spawn_spinner_thread_with_status(None);
    callback(sample_thinking_event());
    // Let the background loop hit at least one timeout tick so the stderr-backed
    // renderer's tick path stays covered under the function threshold.
    std::thread::sleep(std::time::Duration::from_millis(100));
    // stop() sends Done and joins the background thread.
    handle.stop();
}

#[test]
fn test_spawn_spinner_thread_drop_is_best_effort() {
    let (callback, handle) = spawn_spinner_thread_with_status(Some("hdr".into()));
    callback(AgentProgressEvent::ToolStarted {
        tool_call_id: String::new(),
        name: "read".into(),
        arguments: String::new(),
    });
    // Dropping the handle (without stop) exercises the Drop cleanup path.
    drop(callback);
    drop(handle);
}
