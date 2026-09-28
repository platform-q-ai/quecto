//! #2192: a tool that panics costs that one call, never the agent. Its call
//! answers an error naming the crash, the event log records it, the other
//! calls of the response keep their results and the turn goes on.
use super::*;
use crate::application::audit::ports::AuditSink;
use crate::domain::audit::AuditEvent;

#[derive(Debug, Default)]
struct RecordingAudit {
    events: Mutex<Vec<AuditEvent>>,
}

impl AuditSink for RecordingAudit {
    fn emit(
        &self,
        _turn: u32,
        event: AuditEvent,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async move {
            self.events.lock().unwrap().push(event);
            Ok(())
        })
    }
}

/// How a `PanickingTool` panics.
#[derive(Debug, Clone, Copy)]
enum Crash {
    /// Inside its future, after a yield, with a `&'static str` payload.
    InFuture,
    /// Inside its future, with a formatted `String` payload.
    InFutureFormatted,
    /// Before it returns its future at all.
    BeforeFuture,
    /// Inside its future, with a payload that is not text.
    NonTextPayload,
    /// Inside its future, after recording a site on its scope as the
    /// process panic hook does.
    RecordedByHook,
    /// In blocking work it carried, joined through the call-work helper.
    InCarriedWork,
    /// A panic the hook recorded that the tool then swallowed, answering
    /// as if all went well.
    SwallowedAfterRecord,
    /// Inside its future, marked as a contained unwind as the hook does.
    MarkedUnwind,
    /// With a secret-shaped message.
    SecretMessage,
    /// With a message far longer than a result should carry.
    HugeMessage,
}

#[derive(Debug)]
struct PanickingTool {
    name: &'static str,
    crash: Crash,
}

impl Tool for PanickingTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.into(),
            description: "panics".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        let crash = self.crash;
        if matches!(crash, Crash::BeforeFuture) {
            panic!("crashed before the future");
        }
        Box::pin(async move {
            tokio::task::yield_now().await;
            match crash {
                Crash::InFuture => panic!("byte index 2 is not a char boundary"),
                // A value known only at run time, so the payload is a
                // `String` (literal arguments are folded into a `&str`).
                Crash::InFutureFormatted => {
                    let index = std::hint::black_box(7);
                    panic!("index {index} out of range")
                }
                Crash::NonTextPayload => std::panic::panic_any(42_u32),
                Crash::InCarriedWork => {
                    crate::infrastructure::tools::call_work::spawn_blocking_in_call(|| {
                        panic!("carried work panicked")
                    })
                    .await
                    .map_err(|error| DomainError::Tool(error.to_string()))?;
                    unreachable!("the carried panic resumes in the call")
                }
                Crash::MarkedUnwind => {
                    assert!(!crate::application::tool_panic_scope::begin_contained_unwind());
                    panic!("let unwind by the hook")
                }
                Crash::SwallowedAfterRecord => {
                    crate::application::tool_panic_scope::current()
                        .expect("a tool runs inside its call's scope")
                        .record_panic(crate::application::tool_panic_scope::PanicSite {
                            message: "swallowed".into(),
                            location: Some("src/grep.rs:1:1".into()),
                        });
                    Ok(ToolResult {
                        content: "looks fine".into(),
                        is_error: false,
                        image_blocks: vec![],
                        delivery_metadata: None,
                    })
                }
                Crash::RecordedByHook => {
                    let scope = crate::application::tool_panic_scope::current()
                        .expect("a tool runs inside its call's scope");
                    assert_eq!(scope.tool(), "edit_boom");
                    scope.record_panic(crate::application::tool_panic_scope::PanicSite {
                        message: "what the hook saw".into(),
                        location: Some("src/edit.rs:10:5".into()),
                    });
                    panic!("the payload")
                }
                Crash::SecretMessage => {
                    let key = std::hint::black_box("sk-abcdefghijklmnopqrstuv");
                    panic!("request failed with api_key={key} for {key}")
                }
                Crash::HugeMessage => {
                    let long = "é".repeat(std::hint::black_box(4000));
                    panic!("{long}")
                }
                Crash::BeforeFuture => unreachable!("handled above"),
            }
        })
    }
}

/// A tool that answers after a short wait, so it is still running when an
/// overlapping neighbour panics.
#[derive(Debug)]
struct SlowReader;

impl Tool for SlowReader {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_slow".into(),
            description: "slow".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        let content = format!("read {arguments}");
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            Ok(ToolResult {
                content,
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

struct Run {
    /// Each tool result: (content, is_error), in conversation order.
    results: Vec<(String, bool)>,
    /// The final answer of the turn.
    answer: String,
    events: Vec<AuditEvent>,
}

/// Runs one response carrying `calls` (tool names), then a final answer.
async fn run(crash: Crash, calls: &[&str], overlapping: &[&str]) -> Run {
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(PanickingTool {
        name: "edit_boom",
        crash,
    }));
    registry.register(Arc::new(PanickingTool {
        name: "read_boom",
        crash,
    }));
    registry.register(Arc::new(SlowReader));
    registry.overlapping = overlapping.iter().map(|name| name.to_string()).collect();
    let mut response = text_response("");
    response.content = None;
    response.tool_calls = calls
        .iter()
        .enumerate()
        .map(|(i, name)| ToolCall {
            id: format!("call_{i}"),
            name: name.to_string(),
            arguments: format!(r#"{{"n":{i}}}"#),
        })
        .collect();
    let provider = Arc::new(MockProvider::new_results(vec![
        Ok(response),
        Ok(text_response("done")),
    ]));
    let audit = Arc::new(RecordingAudit::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        audit_log: Some(audit.clone()),
        ..test_config(provider, Box::new(registry))
    });
    let mut messages = vec![Message::user("go")];
    agent
        .run_loop(&mut messages)
        .await
        .expect("a panicking tool never ends the turn");
    let results = messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| (m.content.clone(), m.is_error))
        .collect();
    let answer = messages
        .last()
        .filter(|m| m.role == Role::Assistant)
        .map(|m| m.content.clone())
        .unwrap_or_default();
    let events = audit.events.lock().unwrap().clone();
    Run {
        results,
        answer,
        events,
    }
}

fn crashed(tool: &str, message: &str) -> String {
    format!(
        "internal error in tool '{tool}': {message}; the call stopped at the panic, and any \
         partial effects it had already made may remain"
    )
}

#[tokio::test]
async fn a_panicking_tool_answers_an_error_and_the_turn_goes_on() {
    let run = run(Crash::InFuture, &["edit_boom"], &[]).await;
    assert_eq!(
        run.results,
        [(
            crashed("edit_boom", "byte index 2 is not a char boundary"),
            true
        )]
    );
    assert_eq!(run.answer, "done", "the turn continued past the crash");
}

#[tokio::test]
async fn the_event_log_records_the_crash_as_an_error_result() {
    let run = run(Crash::InFuture, &["edit_boom"], &[]).await;
    let recorded: Vec<_> = run
        .events
        .iter()
        .filter_map(|event| match event {
            AuditEvent::ToolResult {
                tool,
                call_id,
                is_error,
                content_preview,
                ..
            } => Some((
                tool.clone(),
                call_id.clone(),
                *is_error,
                content_preview.clone(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        recorded.len(),
        1,
        "one tool_result for the call: {recorded:?}"
    );
    let (tool, call_id, is_error, preview) = &recorded[0];
    assert_eq!(
        (tool.as_str(), call_id.as_str(), *is_error),
        ("edit_boom", "call_0", true)
    );
    assert!(
        preview.contains("internal error in tool 'edit_boom'"),
        "preview names the crash: {preview}"
    );
}

#[tokio::test]
async fn a_sequential_crash_keeps_the_calls_before_and_after_it() {
    // `edit_boom` may not overlap, so the three run one at a time.
    let run = run(
        Crash::InFuture,
        &["read_slow", "edit_boom", "read_slow"],
        &[],
    )
    .await;
    assert_eq!(
        run.results,
        [
            (r#"read {"n":0}"#.to_string(), false),
            (
                crashed("edit_boom", "byte index 2 is not a char boundary"),
                true
            ),
            (r#"read {"n":2}"#.to_string(), false),
        ]
    );
    assert_eq!(run.answer, "done");
}

#[tokio::test]
async fn an_overlapping_crash_keeps_the_other_calls_results() {
    let run = run(
        Crash::InFuture,
        &["read_slow", "read_boom", "read_slow"],
        &["read_slow", "read_boom"],
    )
    .await;
    assert_eq!(
        run.results,
        [
            (r#"read {"n":0}"#.to_string(), false),
            (
                crashed("read_boom", "byte index 2 is not a char boundary"),
                true
            ),
            (r#"read {"n":2}"#.to_string(), false),
        ]
    );
    assert_eq!(run.answer, "done");
}

#[tokio::test]
async fn a_formatted_panic_message_is_reported() {
    let run = run(Crash::InFutureFormatted, &["edit_boom"], &[]).await;
    assert_eq!(
        run.results,
        [(crashed("edit_boom", "index 7 out of range"), true)]
    );
}

#[tokio::test]
async fn a_panic_before_the_tool_returns_its_future_is_contained() {
    let run = run(Crash::BeforeFuture, &["edit_boom"], &[]).await;
    assert_eq!(
        run.results,
        [(crashed("edit_boom", "crashed before the future"), true)]
    );
    assert_eq!(run.answer, "done");
}

#[tokio::test]
async fn a_panic_without_a_text_payload_is_still_reported() {
    let run = run(Crash::NonTextPayload, &["edit_boom"], &[]).await;
    assert_eq!(
        run.results,
        [(crashed("edit_boom", "a panic with no message"), true)]
    );
}

#[tokio::test]
async fn the_hooks_record_names_the_panic_and_the_event_log_keeps_its_location() {
    let run = run(Crash::RecordedByHook, &["edit_boom"], &[]).await;
    assert_eq!(
        run.results,
        [(crashed("edit_boom", "what the hook saw"), true)]
    );
    let errors: Vec<_> = run
        .events
        .iter()
        .filter_map(|event| match event {
            AuditEvent::Error {
                source,
                tool,
                message,
                location,
            } => Some((
                source.as_str(),
                tool.clone(),
                message.as_str(),
                location.clone(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        errors,
        [(
            "tool_panic",
            Some("edit_boom".to_string()),
            "what the hook saw",
            Some("src/edit.rs:10:5".to_string())
        )]
    );
}

#[tokio::test]
async fn without_a_hook_the_payload_names_the_panic_and_no_location_is_claimed() {
    let run = run(Crash::InFuture, &["edit_boom"], &[]).await;
    let error = run
        .events
        .iter()
        .find_map(|event| match event {
            AuditEvent::Error {
                message, location, ..
            } => Some((message.clone(), location.clone())),
            _ => None,
        })
        .expect("an error event");
    assert_eq!(
        error,
        ("byte index 2 is not a char boundary".to_string(), None)
    );
}

#[tokio::test]
async fn a_panic_in_carried_work_fails_its_call_and_the_turn_goes_on() {
    let run = run(Crash::InCarriedWork, &["edit_boom"], &[]).await;
    assert_eq!(
        run.results,
        [(crashed("edit_boom", "carried work panicked"), true)]
    );
    assert_eq!(run.answer, "done");
}

#[tokio::test]
async fn a_panic_the_tool_swallowed_still_fails_its_call() {
    let run = run(Crash::SwallowedAfterRecord, &["edit_boom"], &[]).await;
    assert_eq!(run.results, [(crashed("edit_boom", "swallowed"), true)]);
    assert!(
        run.events.iter().any(|event| matches!(
            event,
            AuditEvent::Error { location: Some(at), .. } if at == "src/grep.rs:1:1"
        )),
        "the swallowed panic is logged with its location"
    );
}

/// A-5: the containment that caught a panic ends its unwind on the thread,
/// so a later first panic there is containable again.
#[tokio::test]
async fn the_containment_that_caught_a_panic_ends_its_unwind() {
    let run = run(Crash::MarkedUnwind, &["edit_boom"], &[]).await;
    assert_eq!(run.results.len(), 1);
    assert!(
        !crate::application::tool_panic_scope::begin_contained_unwind(),
        "no unwind is left in progress on this thread"
    );
    crate::application::tool_panic_scope::end_contained_unwind();
}

/// #2192 review (PRRT_kwDORUxnPM6mf9Bb): a panic's message reaches the
/// provider and the event log with secret shapes redacted.
#[tokio::test]
async fn a_secret_in_a_panic_message_is_redacted_in_the_result_and_the_log() {
    let run = run(Crash::SecretMessage, &["edit_boom"], &[]).await;
    let (content, is_error) = &run.results[0];
    assert!(*is_error);
    assert!(!content.contains("sk-abcdefghijklmnopqrstuv"), "{content}");
    assert!(content.contains("[REDACTED]"), "{content}");
    let logged = run
        .events
        .iter()
        .find_map(|event| match event {
            AuditEvent::Error { message, .. } => Some(message.clone()),
            _ => None,
        })
        .expect("an error event");
    assert!(!logged.contains("sk-abcdefghijklmnopqrstuv"), "{logged}");
    assert!(logged.contains("[REDACTED]"), "{logged}");
}

/// A huge panic message is cut, on a character boundary, before it reaches
/// the provider or the event log.
#[tokio::test]
async fn a_huge_panic_message_is_capped_in_the_result_and_the_log() {
    let run = run(Crash::HugeMessage, &["edit_boom"], &[]).await;
    let (content, _) = &run.results[0];
    assert!(content.len() < 1024 + 300, "{}", content.len());
    assert!(content.contains("é…;"), "cut and marked: {content}");
    let logged = run
        .events
        .iter()
        .find_map(|event| match event {
            AuditEvent::Error { message, .. } => Some(message.clone()),
            _ => None,
        })
        .expect("an error event");
    assert!(logged.len() <= 1024 + "…".len(), "{}", logged.len());
    assert!(logged.ends_with('…'));
}
