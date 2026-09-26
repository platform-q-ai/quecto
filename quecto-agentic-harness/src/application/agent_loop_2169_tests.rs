//! #2169: the tool calls of one response run at once when every one of them
//! may overlap, in order otherwise; their results always come back in call
//! order.
use super::*;

const DELAY: std::time::Duration = std::time::Duration::from_millis(300);

/// A tool that takes `DELAY`, counting how many of its kind run at once.
#[derive(Debug)]
struct SlowTool {
    name: &'static str,
    running: Arc<std::sync::atomic::AtomicUsize>,
    most_at_once: Arc<std::sync::atomic::AtomicUsize>,
}

impl Tool for SlowTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.into(),
            description: "slow".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        use std::sync::atomic::Ordering;
        let content = format!("{} {arguments}", self.name);
        Box::pin(async move {
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.most_at_once.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(DELAY).await;
            self.running.fetch_sub(1, Ordering::SeqCst);
            Ok(ToolResult {
                content,
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

/// Runs one response carrying `calls`, then a final answer; returns the tool
/// results in conversation order and the most calls that ran at once.
async fn run(calls: &[&str]) -> (Vec<String>, usize, std::time::Duration) {
    let running = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let most_at_once = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut registry = MockRegistry::new();
    for name in ["read_slow", "write_slow"] {
        registry.register(Arc::new(SlowTool {
            name,
            running: running.clone(),
            most_at_once: most_at_once.clone(),
        }));
    }
    // Only `read_slow` may overlap.
    registry.overlapping.push("read_slow".to_string());
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
    let mut agent = AgentLoopImpl::new(test_config(provider, Box::new(registry)));
    let mut messages = vec![Message::user("go")];
    let started = std::time::Instant::now();
    agent
        .run_loop(&mut messages)
        .await
        .expect("the turn completes");
    let elapsed = started.elapsed();
    let results = messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| m.content.clone())
        .collect();
    (
        results,
        most_at_once.load(std::sync::atomic::Ordering::SeqCst),
        elapsed,
    )
}

#[tokio::test]
async fn calls_that_may_overlap_run_at_once_and_answer_in_order() {
    let (results, most_at_once, elapsed) = run(&["read_slow", "read_slow", "read_slow"]).await;
    assert_eq!(most_at_once, 3, "the three calls ran at once");
    assert!(elapsed < DELAY * 2, "took {elapsed:?}");
    assert_eq!(
        results,
        [
            r#"read_slow {"n":0}"#,
            r#"read_slow {"n":1}"#,
            r#"read_slow {"n":2}"#
        ]
    );
}

#[tokio::test]
async fn a_call_that_may_not_overlap_keeps_the_batch_in_order() {
    let (results, most_at_once, _) = run(&["read_slow", "write_slow", "read_slow"]).await;
    assert_eq!(most_at_once, 1, "one call at a time");
    assert_eq!(
        results,
        [
            r#"read_slow {"n":0}"#,
            r#"write_slow {"n":1}"#,
            r#"read_slow {"n":2}"#
        ]
    );
}

#[tokio::test]
async fn a_single_call_runs_as_before() {
    let (results, most_at_once, _) = run(&["read_slow"]).await;
    assert_eq!(most_at_once, 1);
    assert_eq!(results, [r#"read_slow {"n":0}"#]);
}
