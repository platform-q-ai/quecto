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
        model: "gpt-5.2".to_string(),
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
