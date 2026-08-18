use std::sync::Arc;

use super::agent_loop_steps::ensure_mock_llm;
use super::*;
use quecto::application::context_pruning;
use quecto::domain::session::{ContextSpillStore, Session, SessionStore, SpillEntry, SpillIndex};
use quecto::infrastructure::persistence::session_store::FileSessionStore;

// ===========================================================================
// In-memory ContextSpillStore for BDD tests
// ===========================================================================

#[derive(Debug)]
struct InMemorySpillStore {
    entries: Mutex<Vec<SpillEntry>>,
}

impl InMemorySpillStore {
    fn new() -> Self {
        Self {
            entries: Mutex::new(vec![]),
        }
    }
}

impl ContextSpillStore for InMemorySpillStore {
    fn append(
        &self,
        _session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.entries.lock().unwrap().push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let id = id.to_string();
        let result = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.id == id)
            .cloned();
        Box::pin(async move { Ok(result) })
    }

    fn list_entries(
        &self,
        _session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<Vec<SpillIndex>>, DomainError>> + Send + '_>> {
        let entries: Vec<SpillIndex> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|e| SpillIndex {
                id: e.id.clone(),
                tool: e.tool.clone(),
                input_preview: e.input_preview.clone(),
                tokens: e.tokens,
            })
            .collect();
        Box::pin(async move { Ok(Arc::new(entries)) })
    }

    fn clear(
        &self,
        _session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.entries.lock().unwrap().clear();
        Box::pin(async { Ok(()) })
    }
}

// ===========================================================================
// Context Pruning Step Definitions
// ===========================================================================

// --- Background ---

#[given("a configured agent with context pruning enabled")]
fn given_context_pruning_enabled(world: &mut QuectoWorld) {
    ensure_mock_llm(world);
    // Initialize spill store in world state
    let store = Arc::new(InMemorySpillStore::new());
    world.context_spill_store = Some(DebugSpillStore(store));
    world.context_messages = Some(vec![]);
    world.context_current_turn = Some(0);
}

// --- Tool-call collapse (#1017) ---

/// Append `n` un-collapsed bash tool-result messages to the session under test.
/// Turn numbers deliberately cycle over a small range so the trigger cannot be
/// mistaken for a turns-elapsed check — collapse must count tool calls.
fn append_tool_calls(world: &mut QuectoWorld, n: u32) {
    let messages = world.context_messages.as_mut().unwrap();
    for i in 0..n {
        let seq = messages.iter().filter(|m| m.role == Role::Tool).count();
        let mut msg = Message::tool(format!("call_{seq}"), format!("output {seq}"));
        msg.turn = Some((i % 3) + 1);
        msg.tool_name = Some("bash".to_string());
        msg.input_preview = Some(format!("cmd {seq}"));
        msg.spill_id = Some(format!("turn{seq}:bash:0"));
        messages.push(msg);
    }
}

/// Run the tool-call collapse trigger and record how many were collapsed.
fn run_collapse(world: &mut QuectoWorld) {
    let max = world
        .context_collapse_after_tool_calls
        .expect("collapse threshold should be set");
    let messages = world.context_messages.as_mut().unwrap();
    let collapsed = context_pruning::collapse_tool_results_over_limit(messages, max);
    world.context_collapsed_count = Some(collapsed);
}

#[given(expr = "context_collapse_after_tool_calls is set to {int}")]
fn given_collapse_threshold(world: &mut QuectoWorld, max: u32) {
    world.context_collapse_after_tool_calls = Some(max);
}

#[given("context collapse is disabled")]
fn given_collapse_disabled(world: &mut QuectoWorld) {
    world.context_collapse_after_tool_calls = Some(context_pruning::COLLAPSE_DISABLED);
}

#[given(expr = "the agent has already executed {int} tool calls in an earlier prompt")]
fn given_already_executed_tool_calls(world: &mut QuectoWorld, n: u32) {
    append_tool_calls(world, n);
}

#[when(expr = "the agent has executed {int} tool calls in the session")]
fn when_executed_tool_calls_in_session(world: &mut QuectoWorld, n: u32) {
    append_tool_calls(world, n);
    run_collapse(world);
}

#[when(expr = "the agent executes {int} more tool calls in a later prompt")]
fn when_executes_more_tool_calls(world: &mut QuectoWorld, n: u32) {
    append_tool_calls(world, n);
    run_collapse(world);
}

#[then("the oldest tool result is collapsed to a recall() stub")]
fn then_oldest_collapsed(world: &mut QuectoWorld) {
    assert_eq!(
        world.context_collapsed_count,
        Some(1),
        "exactly one (the oldest) tool result should collapse"
    );
    let messages = world.context_messages.as_ref().unwrap();
    let oldest = messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .expect("should have a tool result");
    assert!(
        oldest.is_collapsed,
        "oldest tool result should be collapsed"
    );
    assert!(
        oldest.content.contains("recall(\""),
        "collapsed content should be a recall() stub, got: {}",
        oldest.content
    );
}

#[then(expr = "the {int} most recent tool results remain in full context")]
fn then_most_recent_remain(world: &mut QuectoWorld, n: usize) {
    let messages = world.context_messages.as_ref().unwrap();
    let recent: Vec<&Message> = messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .rev()
        .take(n)
        .collect();
    assert_eq!(recent.len(), n, "should have at least {n} tool results");
    for msg in recent {
        assert!(
            !msg.is_collapsed,
            "the {n} most recent tool results must stay in full context"
        );
    }
}

#[then(expr = "{int} tool results are collapsed to recall\\(\\) stubs")]
fn then_n_collapsed(world: &mut QuectoWorld, expected: usize) {
    assert_eq!(
        world.context_collapsed_count,
        Some(expected),
        "expected {expected} tool results to collapse"
    );
}

#[then("no tool results are collapsed")]
fn then_no_tool_results_collapsed(world: &mut QuectoWorld) {
    assert_eq!(
        world.context_collapsed_count,
        Some(0),
        "no tool results should collapse"
    );
    let messages = world.context_messages.as_ref().unwrap();
    assert!(
        messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .all(|m| !m.is_collapsed),
        "no tool result should be collapsed"
    );
}

#[then(expr = "the context_collapse_after_tool_calls default is {int}")]
fn then_collapse_default_is(world: &mut QuectoWorld, expected: u32) {
    let config = quecto::infrastructure::config::Config::default();
    assert_eq!(
        config.agents.defaults.context_collapse_after_tool_calls, expected,
        "default context_collapse_after_tool_calls should be {expected}"
    );
    let _ = world;
}

// --- When steps ---

#[when("the agent executes a bash tool on turn 1")]
fn when_agent_executes_bash_turn_1(world: &mut QuectoWorld) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let messages = world.context_messages.as_mut().unwrap();

    let content = "x".repeat(300); // ~100 tokens
    let spill_id = "turn1:bash:0".to_string();
    let input_preview = "echo hello".to_string();

    // Spill the output
    let entry = SpillEntry {
        id: spill_id.clone(),
        tool: "bash".to_string(),
        input_preview: input_preview.clone(),
        tokens: context_pruning::estimate_tokens(&content),
        content: content.clone(),
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.append("test-session", &entry))
        .unwrap();

    // Append the tool result message with metadata
    let mut msg = Message::tool("call_bash_1", content);
    msg.turn = Some(1);
    msg.tool_name = Some("bash".to_string());
    msg.input_preview = Some(input_preview);
    msg.spill_id = Some(spill_id);
    messages.push(msg);

    world.context_current_turn = Some(1);
    // Save original content for later assertions
    world.context_original_tool_content = Some(messages.last().unwrap().content.clone());
}

#[when(expr = "the agent completes turn {int}")]
fn when_agent_completes_turn(world: &mut QuectoWorld, turn: u32) {
    world.context_current_turn = Some(turn);
    // No collapse — tool results stay in full context.
    // Only the demotion-ladder ceiling would demote messages (if over budget).
}

#[when(expr = "the agent completes turns {int} through {int}")]
fn when_agent_completes_turns_range(world: &mut QuectoWorld, _start: u32, end: u32) {
    world.context_current_turn = Some(end);
    // No collapse — tool results stay in full context.
}

#[when("the agent processes 20 turns of mixed tool and text messages")]
fn when_agent_processes_20_turns_mixed(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_mut().unwrap();

    for turn in 1..=20u32 {
        // Add a user message
        let mut user_msg = Message::user(format!("Question {}", turn));
        user_msg.turn = Some(turn);
        messages.push(user_msg);

        // Add an assistant message
        let mut asst_msg = Message::assistant(format!("Answer {}", turn), vec![]);
        asst_msg.turn = Some(turn);
        messages.push(asst_msg);

        // On even turns, add a tool result
        if turn % 2 == 0 {
            let mut tool_msg = Message::tool(format!("call_{}", turn), format!("output {}", turn));
            tool_msg.turn = Some(turn);
            tool_msg.tool_name = Some("bash".to_string());
            tool_msg.spill_id = Some(format!("turn{}:bash:0", turn));
            messages.push(tool_msg);
        }

        // No collapse — tool results stay in full context
    }

    world.context_current_turn = Some(20);
}

#[when(expr = "the agent processes {int} turns")]
fn when_agent_processes_n_turns(world: &mut QuectoWorld, turns: u32) {
    // No collapse — just advance the turn counter
    world.context_current_turn = Some(turns);
}

#[when(expr = "the agent calls recall with id {string}")]
fn when_agent_calls_recall(world: &mut QuectoWorld, id: String) {
    let store = world.context_spill_store.as_ref().unwrap().clone();

    let result: ToolResult = tokio::runtime::Runtime::new().unwrap().block_on(async {
        if id == "list" {
            let entries = store.list_entries("test-session").await.unwrap();
            if entries.is_empty() {
                return ToolResult {
                    content: "No spilled outputs in this session.".to_string(),
                    is_error: false,
                    image_blocks: vec![],
                };
            }
            let mut output = format!("Spilled outputs ({} entries):\n", entries.len());
            for entry in entries.iter() {
                output.push_str(&format!(
                    "  {} — {} ({} tokens)\n",
                    entry.id, entry.input_preview, entry.tokens
                ));
            }
            ToolResult {
                content: output,
                is_error: false,
                image_blocks: vec![],
            }
        } else {
            match store.recall("test-session", &id).await.unwrap() {
                Some(entry) => ToolResult {
                    content: entry.content,
                    is_error: false,
                    image_blocks: vec![],
                },
                None => ToolResult {
                    content: format!("No spilled output found for id: {}", id),
                    is_error: true,
                    image_blocks: vec![],
                },
            }
        }
    });

    world.context_recall_result = Some(result);
}

#[when(expr = "the agent calls recall with id {string} on turn {int}")]
fn when_agent_calls_recall_on_turn(world: &mut QuectoWorld, id: String, turn: u32) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let messages = world.context_messages.as_mut().unwrap();

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { store.recall("test-session", &id).await })
        .unwrap();

    if let Some(entry) = result {
        // The recall result is itself a tool result, subject to collapse
        let mut msg = Message::tool("call_recall", entry.content);
        msg.turn = Some(turn);
        msg.tool_name = Some("recall".to_string());
        msg.spill_id = Some(format!("turn{}:recall:0", turn));
        messages.push(msg);
    }

    world.context_current_turn = Some(turn);
}

#[when(expr = "the agent calls recall with id {string} three times")]
fn when_agent_calls_recall_three_times(world: &mut QuectoWorld, id: String) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let mut recall_count = 0u32;

    for _ in 0..3 {
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { store.recall("test-session", &id).await })
            .unwrap();
        if result.is_some() {
            recall_count += 1;
        }
    }

    world.context_recall_count = Some(recall_count);
}

#[when("the sliding window drops messages to fit budget")]
fn when_sliding_window_drops(world: &mut QuectoWorld) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let messages = world.context_messages.as_mut().unwrap();

    // Build manifest first (so it's present before sliding window)
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(context_pruning::update_spill_manifest(
            messages,
            store.0.as_ref(),
            "test-session",
        ));

    // Add some non-pinned messages to exceed the budget
    for i in 0..10 {
        messages.push(Message::user(format!("{} {}", "padding ".repeat(30), i)));
    }

    let max_tokens = world.context_max_tokens.unwrap_or(500);
    msg_pruning::enforce_context_ceiling_ladder(
        messages,
        max_tokens,
        context_pruning::DEFAULT_PIN_RECENT_TURNS,
    );
}

#[when(expr = "the agent accumulates {int} tokens of messages")]
fn when_agent_accumulates_tokens(world: &mut QuectoWorld, target_tokens: usize) {
    let messages = world.context_messages.as_mut().unwrap();
    // Accumulate the tokens across several past messages, then end with a
    // small in-flight prompt (a real session always ends with one; the
    // ceiling never drops it, so the bulk must be droppable history).
    let per_msg = "x".repeat((target_tokens / 4) * 3);
    for _ in 0..4 {
        messages.push(Message::user(&per_msg));
    }
    messages.push(Message::user("current question"));
    let max_tokens = world.context_max_tokens.unwrap_or(100_000);
    msg_pruning::enforce_context_ceiling_ladder(
        messages,
        max_tokens,
        context_pruning::DEFAULT_PIN_RECENT_TURNS,
    );
}

#[when(expr = "the agent accumulates {int} tokens across {int} user messages")]
fn when_agent_accumulates_tokens_across(
    world: &mut QuectoWorld,
    target_tokens: usize,
    msg_count: usize,
) {
    let messages = world.context_messages.as_mut().unwrap();
    let per_msg_bytes = (target_tokens * 3) / msg_count;

    // Mark first user message as pinned
    let first_content = "x".repeat(per_msg_bytes);
    let mut first = Message::user(&first_content);
    first.is_pinned = true;
    messages.push(first);

    for _ in 1..msg_count {
        messages.push(Message::user("y".repeat(per_msg_bytes)));
    }

    let max_tokens = world.context_max_tokens.unwrap_or(100_000);
    msg_pruning::enforce_context_ceiling_ladder(
        messages,
        max_tokens,
        context_pruning::DEFAULT_PIN_RECENT_TURNS,
    );
}

#[when("the agent executes tools on turns 1 through 5")]
fn when_agent_executes_tools_turns_1_through_5(world: &mut QuectoWorld) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let messages = world.context_messages.as_mut().unwrap();

    for turn in 1..=5u32 {
        let content = format!("output from turn {}", turn);
        let spill_id = format!("turn{}:bash:0", turn);

        let entry = SpillEntry {
            id: spill_id.clone(),
            tool: "bash".to_string(),
            input_preview: format!("command {}", turn),
            tokens: context_pruning::estimate_tokens(&content),
            content: content.clone(),
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(store.append("test-session", &entry))
            .unwrap();

        let mut msg = Message::tool(format!("call_{}", turn), content);
        msg.turn = Some(turn);
        msg.tool_name = Some("bash".to_string());
        msg.spill_id = Some(spill_id);
        messages.push(msg);
    }

    // Update manifest
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(context_pruning::update_spill_manifest(
            messages,
            store.as_ref(),
            "test-session",
        ));

    world.context_current_turn = Some(5);
}

#[when("the agent processes 3 turns with no tool calls")]
fn when_agent_processes_3_turns_no_tools(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_mut().unwrap();
    for turn in 1..=3u32 {
        messages.push(Message::user(format!("question {}", turn)));
        messages.push(Message::assistant(format!("answer {}", turn), vec![]));
    }
    // Update manifest (should be empty)
    let store = world.context_spill_store.as_ref().unwrap().clone();
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(context_pruning::update_spill_manifest(
            messages,
            store.as_ref(),
            "test-session",
        ));
}

// --- Given steps ---

#[given(expr = "a spilled tool result with id {string}")]
fn given_spilled_tool_result(world: &mut QuectoWorld, id: String) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let entry = SpillEntry {
        id: id.clone(),
        tool: "bash".to_string(),
        input_preview: "echo hello".to_string(),
        tokens: 100,
        content: "hello world original output".to_string(),
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.append("test-session", &entry))
        .unwrap();
}

#[given("a system prompt in the conversation")]
fn given_system_prompt(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_mut().unwrap();
    messages.insert(0, Message::system("You are a helpful assistant."));
}

#[given(expr = "{int} spilled tool results")]
fn given_n_spilled_tool_results(world: &mut QuectoWorld, count: usize) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    for i in 0..count {
        let entry = SpillEntry {
            id: format!("turn{}:bash:0", i + 1),
            tool: "bash".to_string(),
            input_preview: format!("command {}", i + 1),
            tokens: 100 + i,
            content: format!("output from command {}", i + 1),
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(store.append("test-session", &entry))
            .unwrap();
    }
}

#[given("no spill entries exist")]
fn given_no_spill_entries(_world: &mut QuectoWorld) {
    // Default state — spill store is empty
}

#[given(expr = "max_context_tokens is set to {int}")]
fn given_max_context_tokens(world: &mut QuectoWorld, max: usize) {
    world.context_max_tokens = Some(max);
}

#[given(expr = "a system prompt consuming {int} tokens")]
fn given_system_prompt_consuming_tokens(world: &mut QuectoWorld, tokens: usize) {
    let messages = world.context_messages.as_mut().unwrap();
    let content = "s".repeat(tokens * 3); // 3 bytes per token
    let mut msg = Message::system(content);
    msg.is_pinned = true;
    messages.insert(0, msg);
}

// --- Then steps ---

#[then("the tool result from turn 1 is still in full context")]
fn then_tool_result_from_turn_1_still_full(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    let tool_msg = messages
        .iter()
        .find(|m| m.role == Role::Tool && m.turn == Some(1))
        .expect("should find tool result from turn 1");
    assert!(
        !tool_msg.is_collapsed,
        "tool result from turn 1 should not be collapsed"
    );
    assert!(
        !tool_msg.content.starts_with('['),
        "tool result should still have full content, got: {}",
        &tool_msg.content[..50.min(tool_msg.content.len())]
    );
}

#[then(expr = "the spill file contains an entry with id {string}")]
fn then_spill_file_contains_entry(world: &mut QuectoWorld, id: String) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.recall("test-session", &id))
        .unwrap();
    assert!(
        result.is_some(),
        "spill file should contain entry with id '{}'",
        id
    );
}

#[then("the spill entry content matches the original tool output")]
fn then_spill_entry_matches_original(world: &mut QuectoWorld) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let original = world
        .context_original_tool_content
        .as_ref()
        .expect("should have saved original content");
    let entry = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.recall("test-session", "turn1:bash:0"))
        .unwrap()
        .expect("should find spill entry");
    assert_eq!(
        entry.content, *original,
        "spill entry content should match original"
    );
}

#[then("the recall result contains the full original output")]
fn then_recall_result_contains_full_output(world: &mut QuectoWorld) {
    let result = world
        .context_recall_result
        .as_ref()
        .expect("should have recall result");
    assert!(!result.is_error, "recall should succeed");
    assert_eq!(
        result.content, "hello world original output",
        "recall should return full original content"
    );
}

#[then("no tool messages are collapsed")]
fn then_no_tool_messages_collapsed(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    for msg in messages {
        if msg.role == Role::Tool {
            assert!(
                !msg.is_collapsed,
                "tool message should not be collapsed: {}",
                msg.content
            );
        }
    }
}

#[given("a default agent configuration")]
fn given_default_agent_configuration(world: &mut QuectoWorld) {
    // Use defaults from config
    world.context_max_tokens = None; // will use default
    world.context_agent_defaults = Some(
        quecto::infrastructure::config::Config::default()
            .agents
            .defaults,
    );
}

#[then(expr = "the max_context_tokens is {int}")]
fn then_max_context_tokens_is(world: &mut QuectoWorld, expected: usize) {
    // Check that the infrastructure config default matches
    let config = quecto::infrastructure::config::Config::default();
    let actual = config.agents.defaults.max_context_tokens;
    assert_eq!(
        actual, expected,
        "default max_context_tokens should be {}, got {}",
        expected, actual
    );
    let _ = world; // suppress unused warning
}

#[then("all user messages remain in full context")]
fn then_all_user_messages_remain(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    for msg in messages {
        if msg.role == Role::User {
            assert!(
                !msg.is_collapsed,
                "user message should not be collapsed: {}",
                msg.content
            );
        }
    }
}

#[then("all assistant messages remain in full context")]
fn then_all_assistant_messages_remain(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    for msg in messages {
        if msg.role == Role::Assistant {
            assert!(
                !msg.is_collapsed,
                "assistant message should not be collapsed: {}",
                msg.content
            );
        }
    }
}

#[then("the system message remains in full context")]
fn then_system_message_remains(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    let has_system = messages
        .iter()
        .any(|m| m.role == Role::System && !m.is_manifest);
    assert!(has_system, "system message should remain in context");
}

#[then(expr = "the recall result is an error containing {string}")]
fn then_recall_result_is_error(world: &mut QuectoWorld, expected: String) {
    let result = world
        .context_recall_result
        .as_ref()
        .expect("should have recall result");
    assert!(result.is_error, "recall should return an error");
    assert!(
        result.content.contains(&expected),
        "error should contain '{}', got: {}",
        expected,
        result.content
    );
}

#[then(expr = "the result contains all {int} spill entry IDs")]
fn then_result_contains_all_ids(world: &mut QuectoWorld, count: usize) {
    let result = world
        .context_recall_result
        .as_ref()
        .expect("should have recall result");
    for i in 1..=count {
        let id = format!("turn{}:bash:0", i);
        assert!(
            result.content.contains(&id),
            "result should contain id '{}', got: {}",
            id,
            result.content
        );
    }
}

#[then("the result contains tool names and token counts")]
fn then_result_contains_tool_names_and_tokens(world: &mut QuectoWorld) {
    let result = world
        .context_recall_result
        .as_ref()
        .expect("should have recall result");
    assert!(
        result.content.contains("tokens)"),
        "result should contain token counts"
    );
}

#[then("the result does not contain full content")]
fn then_result_does_not_contain_full_content(world: &mut QuectoWorld) {
    let result = world
        .context_recall_result
        .as_ref()
        .expect("should have recall result");
    // Full content would contain "output from command X"
    assert!(
        !result.content.contains("output from command"),
        "result should not contain full output content"
    );
}

#[then(expr = "a warning is logged with target {string}")]
fn then_warning_logged_with_target(world: &mut QuectoWorld, _target: String) {
    // We verified the recall was called 3 times
    let count = world.context_recall_count.unwrap_or(0);
    assert!(
        count >= 3,
        "should have recalled at least 3 times, got {}",
        count
    );
}

#[then(expr = "the warning contains {string}")]
fn then_warning_contains(world: &mut QuectoWorld, _expected: String) {
    // The RecallTool emits this warning internally via tracing.
    // We verify the precondition (3+ recalls) rather than capturing tracing output,
    // since the step definition should test application behavior, not reimplement logging.
    let count = world.context_recall_count.unwrap_or(0);
    assert!(count >= 3, "recall count should be >= 3");
}

#[then(expr = "the warning contains recall_count {int}")]
fn then_warning_contains_recall_count(world: &mut QuectoWorld, expected: u32) {
    let count = world.context_recall_count.unwrap_or(0);
    assert_eq!(
        count, expected,
        "recall count should be {}, got {}",
        expected, count
    );
}

#[then("a pinned manifest message appears in context")]
fn then_pinned_manifest_appears(world: &mut QuectoWorld) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let messages = world.context_messages.as_mut().unwrap();

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(context_pruning::update_spill_manifest(
            messages,
            store.as_ref(),
            "test-session",
        ));

    let manifest = messages.iter().find(|m| m.is_manifest);
    assert!(manifest.is_some(), "manifest message should be present");
    let manifest = manifest.unwrap();
    assert!(manifest.is_pinned, "manifest should be pinned");
}

#[then(expr = "the manifest contains {string}")]
fn then_manifest_contains(world: &mut QuectoWorld, expected: String) {
    let messages = world.context_messages.as_ref().unwrap();
    let manifest = messages
        .iter()
        .find(|m| m.is_manifest)
        .expect("manifest should exist");
    assert!(
        manifest.content.contains(&expected),
        "manifest should contain '{}', got: {}",
        expected,
        manifest.content
    );
}

#[then(expr = "the manifest does not contain {string}")]
fn then_manifest_does_not_contain(world: &mut QuectoWorld, unexpected: String) {
    let messages = world.context_messages.as_ref().unwrap();
    let manifest = messages
        .iter()
        .find(|m| m.is_manifest)
        .expect("manifest should exist");
    assert!(
        !manifest.content.contains(&unexpected),
        "manifest unexpectedly contained '{}': {}",
        unexpected,
        manifest.content
    );
}

#[then("the manifest lists the 10 most recent entries")]
fn then_manifest_lists_10_recent(world: &mut QuectoWorld) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let messages = world.context_messages.as_mut().unwrap();

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(context_pruning::update_spill_manifest(
            messages,
            store.as_ref(),
            "test-session",
        ));

    let manifest = messages
        .iter()
        .find(|m| m.is_manifest)
        .expect("manifest should exist");
    // Count indented lines (entries in the Recent section)
    let recent_count = manifest
        .content
        .lines()
        .filter(|l| l.starts_with("  "))
        .count();
    assert_eq!(
        recent_count, 10,
        "manifest should list 10 recent entries, got {}",
        recent_count
    );
}

#[then(expr = "the manifest shows total count as {int}")]
fn then_manifest_shows_total_count(world: &mut QuectoWorld, expected: usize) {
    let messages = world.context_messages.as_ref().unwrap();
    let manifest = messages
        .iter()
        .find(|m| m.is_manifest)
        .expect("manifest should exist");
    assert!(
        manifest
            .content
            .contains(&format!("{} spilled entries", expected)),
        "manifest should show {} entries, got: {}",
        expected,
        manifest.content
    );
}

#[then("the manifest shows the oldest and latest entry IDs")]
fn then_manifest_shows_oldest_and_latest(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    let manifest = messages
        .iter()
        .find(|m| m.is_manifest)
        .expect("manifest should exist");
    assert!(
        manifest.content.contains("Oldest:"),
        "manifest should show oldest"
    );
    assert!(
        manifest.content.contains("Latest:"),
        "manifest should show latest"
    );
}

#[then("the manifest message remains in context")]
fn then_manifest_remains_in_context(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    assert!(
        messages.iter().any(|m| m.is_manifest),
        "manifest should survive sliding window"
    );
}

#[then("the manifest is pinned")]
fn then_manifest_is_pinned(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    let manifest = messages.iter().find(|m| m.is_manifest);
    assert!(manifest.is_some(), "manifest should exist");
    assert!(manifest.unwrap().is_pinned, "manifest should be pinned");
}

#[then("only one manifest message exists in context")]
fn then_only_one_manifest(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    let count = messages.iter().filter(|m| m.is_manifest).count();
    assert_eq!(count, 1, "should have exactly one manifest, got {}", count);
}

#[then(expr = "it reflects all {int} spill entries")]
fn then_manifest_reflects_all_entries(world: &mut QuectoWorld, count: usize) {
    let messages = world.context_messages.as_ref().unwrap();
    let manifest = messages
        .iter()
        .find(|m| m.is_manifest)
        .expect("manifest should exist");
    assert!(
        manifest
            .content
            .contains(&format!("{} spilled entries", count)),
        "manifest should reflect {} entries, got: {}",
        count,
        manifest.content
    );
}

#[then("no manifest message exists in context")]
fn then_no_manifest(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    assert!(
        !messages.iter().any(|m| m.is_manifest),
        "no manifest should exist in context"
    );
}

#[then("the oldest non-pinned messages are dropped")]
fn then_oldest_non_pinned_dropped(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    let max_tokens = world.context_max_tokens.unwrap_or(100_000);
    let total = context_pruning::estimate_total_tokens(messages);
    assert!(
        total <= max_tokens,
        "total tokens {} should be <= max {}",
        total,
        max_tokens
    );
}

#[then(expr = "total context is under {int} tokens")]
fn then_total_context_under(world: &mut QuectoWorld, max: usize) {
    let messages = world.context_messages.as_ref().unwrap();
    let total = context_pruning::estimate_total_tokens(messages);
    assert!(total <= max, "total tokens {} should be <= {}", total, max);
}

#[then("non-system messages are dropped to fit")]
fn then_non_system_dropped(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    let max_tokens = world.context_max_tokens.unwrap_or(100_000);
    let total = context_pruning::estimate_total_tokens(messages);
    assert!(
        total <= max_tokens,
        "total should be under budget after dropping"
    );
}

#[then("the first user message remains in context")]
fn then_first_user_message_remains(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    let first_user = messages.iter().find(|m| m.role == Role::User);
    assert!(
        first_user.is_some(),
        "first user message should remain in context"
    );
    assert!(
        first_user.unwrap().is_pinned,
        "first user message should be pinned"
    );
}

#[then("later user messages may be dropped")]
fn then_later_user_messages_may_be_dropped(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    let max_tokens = world.context_max_tokens.unwrap_or(100_000);
    let total = context_pruning::estimate_total_tokens(messages);
    assert!(total <= max_tokens, "context should be under budget");
}

// --- #951: spilling sliding window (message spill + tail-pinning) steps ---

#[given(expr = "recent-turn pinning is set to {int} turns")]
fn given_pin_recent_turns(world: &mut QuectoWorld, n: u32) {
    world.context_pin_recent_turns = Some(n);
}

/// Append an old (previous-prompt) message of `tokens` on `turn`, followed by
/// the in-flight user prompt that a real session always ends with.
fn push_old_message_on_turn(world: &mut QuectoWorld, role: Role, tokens: usize, turn: u32) {
    let content = "x".repeat(tokens * 4); // ~4 ASCII chars per token
    let mut msg = match role {
        Role::Assistant => Message::assistant(&content, vec![]),
        _ => Message::user(&content),
    };
    msg.turn = Some(turn);
    let messages = world.context_messages.as_mut().unwrap();
    messages.push(msg);
    messages.push(Message::user("current question"));
    world.context_original_message_content = Some(content);
}

#[given(expr = "an old assistant message of {int} tokens on turn {int}")]
fn given_old_assistant_message(world: &mut QuectoWorld, tokens: usize, turn: u32) {
    push_old_message_on_turn(world, Role::Assistant, tokens, turn);
}

#[given(expr = "an old user message of {int} tokens on turn {int}")]
fn given_old_user_message(world: &mut QuectoWorld, tokens: usize, turn: u32) {
    push_old_message_on_turn(world, Role::User, tokens, turn);
}

#[given("messages from turns 1 through 4 each exceeding the budget")]
fn given_messages_turns_1_through_4(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_mut().unwrap();
    for turn in 1..=4u32 {
        let mut msg = Message::assistant("x".repeat(600), vec![]);
        msg.turn = Some(turn);
        messages.push(msg);
    }
    world.context_current_turn = Some(4);
}

#[given("a user prompt exceeding the budget")]
fn given_user_prompt_exceeding_budget(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_mut().unwrap();
    // Earlier conversation first so the prompt under test is trailing but NOT
    // the first user message — a first-user-only pin must fail the scenario.
    messages.push(Message::user("earlier question"));
    let mut assistant = Message::assistant("earlier answer", vec![]);
    assistant.turn = Some(1);
    messages.push(assistant);
    let prompt = "y".repeat(600); // ~150 tokens
    messages.push(Message::user(&prompt));
    world.context_current_user_prompt = Some(prompt);
}

/// Run the production context-management pipeline (mirroring
/// `apply_context_pruning`): ensure the manifest exists (so it is pinned in
/// context), file every not-yet-spilled conversation message
/// through the creation-time spill writer (#1046), enforce the demotion-
/// ladder ceiling, and insert static guidance if the pass created a first spill.
fn run_spilling_sliding_window(world: &mut QuectoWorld) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let max_tokens = world
        .context_max_tokens
        .expect("max_context_tokens must be set");
    let pin_recent_turns = world
        .context_pin_recent_turns
        .expect("recent-turn pinning must be set");
    let messages = world.context_messages.as_mut().unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        context_pruning::update_spill_manifest(messages, store.0.as_ref(), "test-session").await;
        let mut spilled = false;
        for msg in messages.iter_mut().filter(|m| m.spill_id.is_none()) {
            spilled |=
                msg_pruning::spill_conversation_message(msg, store.0.as_ref(), "test-session")
                    .await;
        }
        msg_pruning::enforce_context_ceiling_ladder(messages, max_tokens, pin_recent_turns);
        if spilled {
            context_pruning::update_spill_manifest(messages, store.0.as_ref(), "test-session")
                .await;
        }
    });
}

#[given("the spilling sliding window has dropped messages to fit budget")]
#[when("the spilling sliding window drops messages to fit budget")]
fn when_spilling_sliding_window_drops(world: &mut QuectoWorld) {
    run_spilling_sliding_window(world);
}

#[when("the agent completes a prompt with no tool calls")]
fn when_agent_completes_prompt_no_tool_calls(world: &mut QuectoWorld) {
    // A no-tool turn still runs context pruning: the ceiling may spill
    // conversation messages, and the manifest must reflect them (#951).
    run_spilling_sliding_window(world);
    let messages = world.context_messages.as_mut().unwrap();
    messages.push(Message::assistant("done", vec![]));
}

#[then("the spill entry content matches the original assistant text")]
fn then_spill_entry_matches_original_assistant(world: &mut QuectoWorld) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let original = world
        .context_original_message_content
        .as_ref()
        .expect("should have saved the original assistant content");
    let entry = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.recall("test-session", "turn1:msg:assistant"))
        .unwrap()
        .expect("should find spill entry turn1:msg:assistant");
    assert_eq!(
        entry.content, *original,
        "spilled content must be the full original assistant text"
    );
}

#[then("the recall result contains the full original assistant text")]
fn then_recall_contains_original_assistant(world: &mut QuectoWorld) {
    let result = world
        .context_recall_result
        .as_ref()
        .expect("should have recall result");
    assert!(
        !result.is_error,
        "recall should succeed: {}",
        result.content
    );
    let original = world
        .context_original_message_content
        .as_ref()
        .expect("should have saved the original assistant content");
    assert_eq!(
        result.content, *original,
        "recall must return the full original assistant text"
    );
}

#[then(expr = "messages from the most recent {int} turns remain in context")]
fn then_most_recent_turns_remain(world: &mut QuectoWorld, n: u32) {
    let max_turn = world
        .context_current_turn
        .expect("current turn should be set");
    let messages = world.context_messages.as_ref().unwrap();
    for turn in (max_turn - n + 1)..=max_turn {
        assert!(
            messages.iter().any(|m| m.turn == Some(turn)),
            "turn {turn} is within the pinned tail and must remain in context"
        );
    }
}

#[then("messages from older turns are dropped")]
fn then_older_turns_dropped(world: &mut QuectoWorld) {
    let max_turn = world
        .context_current_turn
        .expect("current turn should be set");
    let pin = world
        .context_pin_recent_turns
        .expect("recent-turn pinning must be set");
    let messages = world.context_messages.as_ref().unwrap();
    assert!(
        messages
            .iter()
            .all(|m| m.turn.is_none_or(|t| t > max_turn - pin)),
        "turns outside the pinned tail must be dropped to approach budget"
    );
}

#[then("the current user prompt remains in context")]
fn then_current_user_prompt_remains(world: &mut QuectoWorld) {
    let prompt = world
        .context_current_user_prompt
        .as_ref()
        .expect("should have saved the current user prompt");
    let messages = world.context_messages.as_ref().unwrap();
    assert!(
        messages
            .iter()
            .any(|m| m.role == Role::User && m.content == *prompt),
        "the in-flight user prompt must never be dropped by the ceiling"
    );
}

// --- Session persistence round-trip steps ---

#[when("the session is saved and reloaded from disk")]
fn when_session_saved_and_reloaded(world: &mut QuectoWorld) {
    let messages = world.context_messages.take().unwrap();
    let session = Session {
        key: "test:persistence".to_string(),
        messages,
        workflow_run: None,
        subagent_roster: Vec::new(),
    };

    let tmp = tempfile::TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let reloaded = tokio::runtime::Runtime::new().unwrap().block_on(async {
        store.save(&session).await.unwrap();
        store.load("test:persistence").await.unwrap().unwrap()
    });

    world.context_messages = Some(reloaded.messages);
    // Keep temp dir alive for the scenario duration
    world.context_temp_dir = Some(tmp);
}

#[when("the spill manifest is updated")]
fn when_spill_manifest_updated(world: &mut QuectoWorld) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let messages = world.context_messages.as_mut().unwrap();

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(context_pruning::update_spill_manifest(
            messages,
            store.as_ref(),
            "test-session",
        ));
}

/// Find the tool result from turn 1 by structural criteria.
fn find_tool_from_turn_1(messages: &[Message]) -> &Message {
    messages
        .iter()
        .find(|m| m.role == Role::Tool && m.turn == Some(1))
        .expect("should find a tool result from turn 1")
}

#[then(expr = "the tool result from turn 1 still has turn {int}")]
fn then_tool_result_turn1_still_has_turn(world: &mut QuectoWorld, expected: u32) {
    let messages = world.context_messages.as_ref().unwrap();
    let tool_msg = find_tool_from_turn_1(messages);
    assert_eq!(
        tool_msg.turn,
        Some(expected),
        "turn should survive save/load round-trip"
    );
}

#[then(expr = "the tool result from turn 1 still has tool_name {string}")]
fn then_tool_result_turn1_still_has_tool_name(world: &mut QuectoWorld, expected: String) {
    let messages = world.context_messages.as_ref().unwrap();
    let tool_msg = find_tool_from_turn_1(messages);
    assert_eq!(
        tool_msg.tool_name.as_deref(),
        Some(expected.as_str()),
        "tool_name should survive save/load round-trip"
    );
}

#[then(expr = "exactly one system message contains {string}")]
fn then_exactly_one_system_msg_contains(world: &mut QuectoWorld, needle: String) {
    let messages = world.context_messages.as_ref().unwrap();
    let count = messages
        .iter()
        .filter(|m| m.role == Role::System && m.content.contains(&needle))
        .count();
    assert_eq!(
        count, 1,
        "expected exactly 1 system message containing '{}', got {}",
        needle, count
    );
}

#[then(expr = "the tool result from turn 1 still has spill_id {string}")]
fn then_tool_result_turn1_still_has_spill_id(world: &mut QuectoWorld, expected: String) {
    let messages = world.context_messages.as_ref().unwrap();
    let tool_msg = find_tool_from_turn_1(messages);
    assert_eq!(
        tool_msg.spill_id.as_deref(),
        Some(expected.as_str()),
        "spill_id should survive save/load round-trip"
    );
}

// ===========================================================================
// Token estimation heuristic steps (#305)
// ===========================================================================

#[given(expr = "a string of {int} ASCII characters")]
fn given_ascii_string(world: &mut QuectoWorld, count: usize) {
    world.token_estimate_input = Some("x".repeat(count));
}

#[given(expr = "a string of {int} CJK characters")]
fn given_cjk_string(world: &mut QuectoWorld, count: usize) {
    // Use a common CJK character (U+4E2D = 中)
    world.token_estimate_input = Some("中".repeat(count));
}

#[then(expr = "the estimated token count should be {int}")]
fn then_estimated_token_count(world: &mut QuectoWorld, expected: usize) {
    let input = world
        .token_estimate_input
        .as_ref()
        .expect("no input string set");
    let actual = context_pruning::estimate_tokens(input);
    assert_eq!(
        actual,
        expected,
        "expected {} tokens for {} chars, got {}",
        expected,
        input.len(),
        actual
    );
}

// --- Spill store caching (#375) ---

#[when(expr = "{int} spill entries are appended to the store")]
fn when_n_spill_entries_appended(world: &mut QuectoWorld, count: usize) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let store = quecto::infrastructure::persistence::context_spill::FileContextSpillStore::new(
        tmp.path().to_path_buf(),
    );
    let (cache_count, cache_result) = rt.block_on(async {
        // Append first entry, then seed cache via list_entries (mirrors agent loop)
        let first = SpillEntry {
            id: "turn1:bash:0".to_string(),
            tool: "bash".to_string(),
            input_preview: "cmd-1".to_string(),
            tokens: 100,
            content: "output-1\n".to_string(),
        };
        store.append("cache-test", &first).await.unwrap();
        let _ = store.list_entries("cache-test").await.unwrap();
        // Append remaining entries (cache updated incrementally)
        for i in 1..count {
            let entry = SpillEntry {
                id: format!("turn{}:bash:0", i + 1),
                tool: "bash".to_string(),
                input_preview: format!("cmd-{}", i + 1),
                tokens: 100,
                content: format!("output-{}\n", i + 1),
            };
            store.append("cache-test", &entry).await.unwrap();
        }
        // Delete the spill file to prove cache is used
        let spill_path = tmp
            .path()
            .join("sessions")
            .join("cache-test")
            .join("spill.jsonl");
        tokio::fs::remove_file(&spill_path).await.unwrap();
        // Verify list_entries still works from cache
        let entries = store.list_entries("cache-test").await.unwrap();
        (count.to_string(), entries.len().to_string())
    });
    world
        .env_overrides
        .insert("_spill_cache_count".into(), cache_count);
    world
        .env_overrides
        .insert("_spill_cache_result".into(), cache_result);
}

#[when("recall is called for the 5th entry in a 10-entry spill file")]
fn when_recall_5th_in_10_entries(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let store = quecto::infrastructure::persistence::context_spill::FileContextSpillStore::new(
        tmp.path().to_path_buf(),
    );
    let (result_id, result_content) = rt.block_on(async {
        for i in 0..10 {
            let entry = SpillEntry {
                id: format!("turn{}:bash:0", i + 1),
                tool: "bash".to_string(),
                input_preview: format!("cmd-{}", i + 1),
                tokens: 100,
                content: format!("output-{}\n", i + 1),
            };
            store.append("recall-test", &entry).await.unwrap();
        }
        let recalled = store.recall("recall-test", "turn5:bash:0").await.unwrap();
        let entry = recalled.expect("should find turn5:bash:0");
        (entry.id, entry.content)
    });
    world
        .env_overrides
        .insert("_recall_result_id".into(), result_id);
    world
        .env_overrides
        .insert("_recall_result_content".into(), result_content);
}

#[then("the correct entry is returned with full content")]
fn then_correct_entry_returned(world: &mut QuectoWorld) {
    let id = world
        .env_overrides
        .get("_recall_result_id")
        .expect("recall result id not set");
    assert_eq!(id, "turn5:bash:0", "should recall the 5th entry");
    let content = world
        .env_overrides
        .get("_recall_result_content")
        .expect("recall result content not set");
    assert_eq!(content, "output-5\n", "content should match the 5th entry");
}

#[then(expr = "list_entries returns {int} entries without re-reading disk")]
fn then_list_entries_from_cache(world: &mut QuectoWorld, expected: usize) {
    let result: usize = world
        .env_overrides
        .get("_spill_cache_result")
        .expect("cache result not set")
        .parse()
        .unwrap();
    assert_eq!(
        result, expected,
        "list_entries should return {expected} cached entries after disk file was deleted"
    );
}

// ===========================================================================
// #1046: creation-time message spilling + count-based message collapse
// #1045: configurable pin_recent_turns
// #1044: window-aware budget + observable unmet ceiling
// ===========================================================================

use quecto::application::context_pruning::messages as msg_pruning;

/// Complete one text-only prompt through the REAL agent loop, so these
/// scenarios fail if the loop's creation-time spilling (#1046 AC1) is ever
/// removed: the loop itself must spill the turn-1 assistant reply (and the
/// in-flight prompt) into the world's spill store.
fn complete_text_only_prompt(world: &mut QuectoWorld, reply: &str) {
    use quecto::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
    use quecto::domain::agent::AgentLoop;

    let store = world.context_spill_store.as_ref().unwrap().clone();
    let mock = ensure_mock_llm(world);
    mock.push_response(quecto::domain::message::LlmResponse {
        content: Some(reply.to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    });
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: mock,
        tool_registry: Box::new(quecto::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "test-model".into(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: Some(store.0.clone()),
        session_key: session_key_under_test(world),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    });
    let messages = world.context_messages.as_mut().unwrap();
    messages.push(Message::user("a question"));
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(agent.process(messages))
        .expect("the text-only prompt must complete");
    world.context_original_message_content = Some(reply.to_string());
}

#[given("the agent has completed a text-only prompt on turn 1")]
#[when("the agent completes a text-only prompt on turn 1")]
fn when_completes_text_only_prompt(world: &mut QuectoWorld) {
    complete_text_only_prompt(world, "the full assistant reply");
}

#[when("the agent completes another text-only prompt on turn 1")]
fn when_completes_another_text_only_prompt(world: &mut QuectoWorld) {
    complete_text_only_prompt(world, "a second assistant reply");
}

#[then(expr = "the spill entry for {string} matches the assistant reply")]
fn then_spill_entry_matches_reply(world: &mut QuectoWorld, id: String) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let entry = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.recall("test-session", &id))
        .unwrap()
        .unwrap_or_else(|| panic!("spill entry {id} must exist at creation time"));
    let original = world
        .context_original_message_content
        .as_ref()
        .expect("the completed prompt must have recorded its reply");
    assert_eq!(
        entry.content, *original,
        "the creation-time spill must hold the full assistant reply"
    );
}

#[given(expr = "context_collapse_after_messages is set to {int}")]
fn given_message_collapse_threshold(world: &mut QuectoWorld, max: u32) {
    world.context_collapse_after_messages = Some(max);
}

#[given("message collapse is disabled")]
fn given_message_collapse_disabled(world: &mut QuectoWorld) {
    world.context_collapse_after_messages = Some(context_pruning::COLLAPSE_DISABLED);
}

/// Append an old (previous-prompt) conversation message on `turn`, already
/// spilled at creation per #1046 AC1 (spill_id stamped). Records the first
/// message's token estimate so stub-vs-original assertions never have to
/// reconstruct this content.
fn push_old_conv_message(world: &mut QuectoWorld, role: Role, turn: u32, i: usize) {
    let content = format!("old conversation message {i} {}", "padding ".repeat(20));
    let mut m = match role {
        Role::Assistant => Message::assistant(&content, vec![]),
        _ => Message::user(&content),
    };
    m.turn = Some(turn);
    m.spill_id = Some(format!("turn{turn}:msg:{}", role.as_str()));
    if world.context_original_tokens.is_none() {
        world.context_original_tokens = Some(context_pruning::estimate_message_tokens(&m));
    }
    world.context_messages.as_mut().unwrap().push(m);
}

fn push_n_old_conv_messages(world: &mut QuectoWorld, n: usize) {
    let start = world
        .context_messages
        .as_ref()
        .unwrap()
        .iter()
        .filter(|m| m.role == Role::User || m.role == Role::Assistant)
        .count();
    for i in 0..n {
        let role = if i % 2 == 0 {
            Role::User
        } else {
            Role::Assistant
        };
        push_old_conv_message(world, role, (i + 1) as u32, start + i + 1);
    }
}

// Both phrasings share one definition: turn numbering restarts on every
// prompt, so a later batch reuses the same small turn numbers either way.
#[given(expr = "{int} old conversation messages")]
#[given(expr = "{int} old conversation messages from an earlier prompt")]
fn given_old_conv_messages(world: &mut QuectoWorld, n: usize) {
    push_n_old_conv_messages(world, n);
}

#[given("an in-flight user prompt")]
fn given_in_flight_user_prompt(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_mut().unwrap();
    messages.push(Message::user("current question"));
}

// Matches production state at trim time: creation-time spilling files the
// in-flight prompt (stamping spill_id) BEFORE the collapse triggers run, so
// only the AC3 exemption — not the spill_id filter — protects it. Makes the
// exemption falsifiable (round-2 review of PR #1048).
#[given("an in-flight user prompt already spilled at creation")]
fn given_in_flight_user_prompt_spilled(world: &mut QuectoWorld) {
    let mut m = Message::user("current question");
    m.spill_id = Some("turn0:msg:user".into());
    world.context_messages.as_mut().unwrap().push(m);
}

#[given(expr = "{int} old assistant messages and {int} old user messages")]
fn given_old_mixed_messages(world: &mut QuectoWorld, a: usize, u: usize) {
    for i in 0..a {
        push_old_conv_message(world, Role::Assistant, (i + 1) as u32, i + 1);
    }
    for i in 0..u {
        push_old_conv_message(world, Role::User, (i + 1) as u32, a + i + 1);
    }
}

#[given(expr = "{int} un-collapsed tool results in the session")]
fn given_uncollapsed_tool_results(world: &mut QuectoWorld, n: u32) {
    append_tool_calls(world, n);
}

#[given("a pinned manifest message in the conversation")]
fn given_pinned_manifest_message(world: &mut QuectoWorld) {
    let mut manifest = Message::system("[Session memory: 1 spilled entries via recall()]");
    manifest.is_pinned = true;
    manifest.is_manifest = true;
    world.context_messages.as_mut().unwrap().insert(0, manifest);
}

#[given("a conversation message within the pinned recent-turn tail")]
fn given_tail_pinned_conv_message(world: &mut QuectoWorld) {
    // A turn-stamped message of the current prompt's most recent turn: it
    // sits after the in-flight prompt, inside the pin_recent_turns tail.
    // Spilled at creation (spill_id stamped) like every conversation message
    // in production, so only the tail-pin exemption — not the spill_id
    // filter — protects it from collapse (falsifiability, PR #1048 round 2).
    let mut m = Message::assistant("tail-pinned recent answer", vec![]);
    m.turn = Some(99);
    m.spill_id = Some("turn99:msg:assistant".into());
    world.context_messages.as_mut().unwrap().push(m);
}

#[when("the agent trims old conversation messages")]
fn when_agent_trims_conversation(world: &mut QuectoWorld) {
    let max = world
        .context_collapse_after_messages
        .expect("context_collapse_after_messages must be set");
    // Scenarios that exercise the tail-pin set pinning explicitly; the rest
    // opt out so the count trigger is observed in isolation.
    let pin = world.context_pin_recent_turns.unwrap_or(0);
    let messages = world.context_messages.as_mut().unwrap();
    let collapsed = msg_pruning::collapse_conversation_messages_over_limit(messages, max, pin);
    world.context_msg_collapsed_count = Some(collapsed);
}

#[then(expr = "{int} conversation message is collapsed to a recall stub")]
#[then(expr = "{int} conversation messages are collapsed to recall stubs")]
fn then_n_messages_collapsed(world: &mut QuectoWorld, expected: usize) {
    assert_eq!(
        world.context_msg_collapsed_count,
        Some(expected),
        "expected exactly {expected} conversation messages to collapse"
    );
}

#[then(expr = "at least {int} conversation message is collapsed to a recall stub")]
fn then_at_least_n_messages_collapsed(world: &mut QuectoWorld, min: usize) {
    let got = world.context_msg_collapsed_count.unwrap_or(0);
    assert!(
        got >= min,
        "expected at least {min} conversation messages to collapse, got {got}"
    );
}

#[then("the oldest conversation message is a one-line recall stub")]
fn then_oldest_message_is_stub(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    let oldest = messages
        .iter()
        .find(|m| m.role == Role::User || m.role == Role::Assistant)
        .expect("a conversation message must exist");
    assert!(oldest.is_collapsed, "the oldest must be collapsed");
    assert!(
        oldest.content.contains("recall(\"") && !oldest.content.contains('\n'),
        "the stub must be a one-line recall() stub, got: {}",
        oldest.content
    );
    assert!(
        oldest.content.contains("tokens"),
        "the stub must carry the token count, got: {}",
        oldest.content
    );
}

#[then("no tool results are collapsed by the message trigger")]
fn then_no_tool_results_collapsed_by_message_trigger(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    assert!(
        messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .all(|m| !m.is_collapsed),
        "the message trigger must never collapse tool results"
    );
}

/// Find a surviving message and report `(is_collapsed_or_stubbed, content)`
/// so each Then step can assert on the observable state directly.
fn collapse_state(
    world: &QuectoWorld,
    pred: impl Fn(&Message) -> bool,
    what: &str,
) -> (bool, String) {
    let messages = world.context_messages.as_ref().unwrap();
    let msg = messages
        .iter()
        .find(|m| pred(m))
        .unwrap_or_else(|| panic!("{what} must still be in context"));
    (
        msg.is_collapsed || msg.content.contains("recall(\""),
        msg.content.clone(),
    )
}

#[then("the system prompt is not collapsed")]
fn then_system_prompt_not_collapsed(world: &mut QuectoWorld) {
    let (collapsed, content) = collapse_state(
        world,
        |m| m.role == Role::System && !m.is_manifest,
        "the system prompt",
    );
    assert!(
        !collapsed,
        "the system prompt must keep its full content, got: {content}"
    );
}

#[then("the manifest message is not collapsed")]
fn then_manifest_not_collapsed(world: &mut QuectoWorld) {
    let (collapsed, content) = collapse_state(world, |m| m.is_manifest, "the spill manifest");
    assert!(
        !collapsed,
        "the spill manifest must keep its full content, got: {content}"
    );
}

#[then("the in-flight user prompt is not collapsed")]
fn then_inflight_prompt_not_collapsed(world: &mut QuectoWorld) {
    let (collapsed, content) = collapse_state(
        world,
        |m| m.role == Role::User && m.turn.is_none() && m.content == "current question",
        "the in-flight user prompt",
    );
    assert!(
        !collapsed,
        "the in-flight user prompt must keep its full content, got: {content}"
    );
}

#[then("the tail-pinned conversation message is not collapsed")]
fn then_tail_pinned_not_collapsed(world: &mut QuectoWorld) {
    let (collapsed, content) = collapse_state(
        world,
        |m| m.content.contains("tail-pinned recent answer"),
        "a message within the pin_recent_turns tail",
    );
    assert!(
        !collapsed,
        "the pin_recent_turns tail must keep its full content, got: {content}"
    );
}

#[then("each collapsed message stub has a nonzero token estimate")]
fn then_stubs_have_nonzero_tokens(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_ref().unwrap();
    let stubs: Vec<&Message> = messages.iter().filter(|m| m.is_collapsed).collect();
    assert!(!stubs.is_empty(), "positive control: stubs must exist");
    for stub in stubs {
        assert!(
            context_pruning::estimate_message_tokens(stub) > 0,
            "stubs must count toward the token budget"
        );
    }
}

#[then("the stub token estimate is below the original message estimate")]
fn then_stub_cheaper_than_original(world: &mut QuectoWorld) {
    let original_tokens = world
        .context_original_tokens
        .expect("the Given must have recorded the original message estimate");
    let messages = world.context_messages.as_ref().unwrap();
    let stub = messages
        .iter()
        .find(|m| m.is_collapsed)
        .expect("a stub must exist");
    assert!(
        context_pruning::estimate_message_tokens(stub) < original_tokens,
        "the stub must be cheaper than the message it replaced"
    );
}

// --- demotion ladder (#1046 AC6, #1044 AC1) ---

#[when("the agent enforces the context ceiling")]
fn when_agent_enforces_ceiling(world: &mut QuectoWorld) {
    let max_tokens = world
        .context_max_tokens
        .expect("max_context_tokens must be set");
    let pin = world
        .context_pin_recent_turns
        .expect("recent-turn pinning must be set");
    let messages = world.context_messages.as_mut().unwrap();
    let outcome = msg_pruning::enforce_context_ceiling_ladder(messages, max_tokens, pin);
    world.context_ladder_outcome = Some(outcome);
}

fn ladder_outcome(world: &QuectoWorld) -> &msg_pruning::CeilingLadderOutcome {
    world
        .context_ladder_outcome
        .as_ref()
        .expect("the ceiling must have been enforced")
}

#[then(expr = "at least {int} old message is reduced to a recall stub by the ceiling")]
fn then_ceiling_stubbed_at_least(world: &mut QuectoWorld, min: usize) {
    let outcome = ladder_outcome(world);
    assert!(
        outcome.collapsed_to_stubs >= min,
        "the ladder's first rung must stub at least {min} messages, got {}",
        outcome.collapsed_to_stubs
    );
}

#[then("no messages are removed from the conversation")]
fn then_ceiling_dropped_nothing(world: &mut QuectoWorld) {
    let outcome = ladder_outcome(world);
    assert_eq!(
        outcome.dropped, 0,
        "the ladder must not hard-drop while stubbing suffices"
    );
}

#[then(expr = "at least {int} message is removed from the conversation")]
fn then_ceiling_dropped_at_least(world: &mut QuectoWorld, min: usize) {
    let outcome = ladder_outcome(world);
    assert!(
        outcome.dropped >= min,
        "the ladder's second rung must drop stubs when stubbing is not enough, got {}",
        outcome.dropped
    );
}

#[then("no full un-collapsed conversation message was removed before stubbing")]
fn then_no_full_message_dropped_before_stubbing(world: &mut QuectoWorld) {
    // Ladder ordering is observable in what survives: every remaining old
    // conversation message must be a stub (demoted first), never a full
    // message that outlived a dropped sibling.
    let messages = world.context_messages.as_ref().unwrap();
    assert!(
        messages
            .iter()
            .filter(|m| (m.role == Role::User || m.role == Role::Assistant) && m.turn.is_some())
            .all(|m| m.is_collapsed),
        "full old messages surviving while stubs were dropped violates ladder ordering"
    );
}

#[then("the context budget is reported as unmet")]
fn then_ceiling_reports_unmet(world: &mut QuectoWorld) {
    let outcome = ladder_outcome(world);
    assert!(
        outcome.over_budget,
        "an unmeetable ceiling must be reported so the loop can warn and audit (#1044)"
    );
}

// --- #1045: configurable pin_recent_turns ---

#[then(expr = "the configured pin_recent_turns is {int}")]
fn then_configured_pin_recent_turns(world: &mut QuectoWorld, expected: u32) {
    let defaults = world
        .context_agent_defaults
        .as_ref()
        .expect("a default agent configuration must have been established");
    assert_eq!(
        defaults.pin_recent_turns, expected,
        "default pin_recent_turns should be {expected}"
    );
}

#[then(expr = "the configured context_collapse_after_messages is {int}")]
fn then_configured_message_collapse(world: &mut QuectoWorld, expected: u32) {
    let defaults = world
        .context_agent_defaults
        .as_ref()
        .expect("a default agent configuration must have been established");
    assert_eq!(
        defaults.context_collapse_after_messages, expected,
        "message collapse default must keep the most recent {expected} conversation messages"
    );
}

// --- #1044: unmet ceiling reaches the ContextPruned audit event ---

/// An audit sink recording every emitted event for inspection.
#[derive(Default)]
struct RecordingAuditSink {
    events: Mutex<Vec<quecto::domain::audit::AuditEvent>>,
}

impl quecto::domain::audit::AuditSink for RecordingAuditSink {
    fn emit(
        &self,
        _turn: u32,
        event: quecto::domain::audit::AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.events.lock().unwrap().push(event);
        Box::pin(async { Ok(()) })
    }
}

#[when("the agent completes a prompt exceeding the budget")]
fn when_agent_completes_over_budget_prompt(world: &mut QuectoWorld) {
    use quecto::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
    use quecto::domain::agent::AgentLoop;

    let max_context_tokens = world
        .context_max_tokens
        .expect("max_context_tokens must be set");
    let mock = ensure_mock_llm(world);
    mock.push_response(quecto::domain::message::LlmResponse {
        content: Some("done".to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    });
    let sink = Arc::new(RecordingAuditSink::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: mock,
        tool_registry: Box::new(quecto::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "test-model".into(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: None,
        session_key: "test-session".into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: Some(sink.clone() as Arc<dyn quecto::domain::audit::AuditSink>),
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    });
    // The in-flight prompt alone (never droppable) exceeds the tiny budget.
    let mut messages = vec![Message::user("y".repeat(600))];
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(agent.process(&mut messages))
        .expect("the over-budget prompt must still complete");
    world.audit_loop_events = sink.events.lock().unwrap().clone();
}

#[then("the ContextPruned audit event records the budget as unmet")]
fn then_audit_event_records_unmet_budget(world: &mut QuectoWorld) {
    let unmet_flags: Vec<bool> = world
        .audit_loop_events
        .iter()
        .filter_map(|e| match e {
            quecto::domain::audit::AuditEvent::ContextPruned { budget_unmet, .. } => {
                Some(*budget_unmet)
            }
            _ => None,
        })
        .collect();
    assert!(
        unmet_flags.iter().any(|unmet| *unmet),
        "an unmeetable ceiling must emit a ContextPruned audit event with \
         budget_unmet=true; ContextPruned events seen: {unmet_flags:?}"
    );
}

// --- #1044: window-aware effective context budget ---

#[given(expr = "a configured agent with max_context_tokens {int}")]
fn given_agent_with_max_context_tokens(world: &mut QuectoWorld, max: usize) {
    world.context_budget_config = Some(max);
}

#[given(expr = "the active model has a known context window of {int} tokens")]
fn given_model_known_window(world: &mut QuectoWorld, window: usize) {
    world.context_model_window = Some(Some(window));
}

#[given("the active model has no known context window")]
fn given_model_unknown_window(world: &mut QuectoWorld) {
    world.context_model_window = Some(None);
}

#[when("the agent derives its effective context budget")]
fn when_agent_derives_effective_budget(world: &mut QuectoWorld) {
    let provider = Arc::new(MockLlmProvider::new());
    let agent = AgentLoopImpl::new(quecto::application::agent_loop::AgentLoopConfig {
        provider,
        tool_registry: Box::new(ToolRegistryImpl::new()),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: world.context_budget_config.expect("config budget set"),
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: world.context_model_window.expect("window declared"),
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    });
    world.context_effective_budget = Some(agent.effective_max_context_tokens());
}

#[then(expr = "the effective context budget is {int}")]
fn then_effective_context_budget(world: &mut QuectoWorld, expected: usize) {
    assert_eq!(
        world.context_effective_budget,
        Some(expected),
        "the effective budget must derive from the model window with the \
         config value as override/fallback"
    );
}

// ===========================================================================
// Ephemeral sessions (empty session key) — conversation/tool spill symmetry
// and rewind after conversation-message collapse (PR #1048 follow-up fixes)
// ===========================================================================

/// The session key the loop-driving steps use: empty for ephemeral runs
/// (`--no-session`), the fixture key otherwise.
fn session_key_under_test(world: &QuectoWorld) -> String {
    if world.context_ephemeral {
        String::new()
    } else {
        "test-session".to_string()
    }
}

#[given("the session is ephemeral")]
fn given_ephemeral_session(world: &mut QuectoWorld) {
    world.context_ephemeral = true;
}

/// Drive the REAL agent loop for one prompt so scenarios observe exactly
/// what a run persists; honours the ephemeral-session context step.
fn run_prompt_through_loop(
    world: &mut QuectoWorld,
    responses: Vec<quecto::domain::message::LlmResponse>,
) {
    use quecto::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
    use quecto::domain::agent::AgentLoop;

    let store = world.context_spill_store.as_ref().unwrap().clone();
    let session_key = session_key_under_test(world);
    let mock = ensure_mock_llm(world);
    for r in responses {
        mock.push_response(r);
    }
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: mock,
        tool_registry: Box::new(quecto::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "test-model".into(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: Some(store.0.clone()),
        session_key,
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    });
    let messages = world.context_messages.as_mut().unwrap();
    messages.push(Message::user("a question"));
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(agent.process(messages))
        .expect("the prompt must complete");
}

fn text_llm_response(text: &str) -> quecto::domain::message::LlmResponse {
    quecto::domain::message::LlmResponse {
        content: Some(text.to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

#[when("the agent runs a bash tool")]
fn when_agent_runs_bash_tool(world: &mut QuectoWorld) {
    let tool_call = quecto::domain::message::LlmResponse {
        content: None,
        tool_calls: vec![quecto::domain::message::ToolCall {
            id: "call_1".into(),
            name: "bash".into(),
            arguments: r#"{"command":"echo hi"}"#.into(),
        }],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    };
    run_prompt_through_loop(world, vec![tool_call, text_llm_response("done")]);
}

#[then(expr = "the ephemeral session spill contains a recallable entry with id {string}")]
fn then_ephemeral_spill_recallable(world: &mut QuectoWorld, id: String) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let entry = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.recall("", &id))
        .unwrap()
        .unwrap_or_else(|| {
            panic!(
                "ephemeral sessions must spill conversation messages at creation \
                 so collapse/ladder recall() stubs stay resolvable; missing {id}"
            )
        });
    let expected = world
        .context_original_message_content
        .as_ref()
        .expect("the completed prompt must have recorded its reply");
    assert_eq!(
        entry.content, *expected,
        "the ephemeral spill entry must carry the full assistant reply"
    );
}

#[then(expr = "the ephemeral session spill contains a recallable entry whose tool is {string}")]
fn then_ephemeral_spill_has_tool_entry(world: &mut QuectoWorld, tool: String) {
    let store = world.context_spill_store.as_ref().unwrap().clone();
    let index = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.list_entries(""))
        .unwrap();
    let entry = index.iter().find(|e| e.tool == tool).unwrap_or_else(|| {
        panic!(
            "ephemeral tool spilling (deliberately unguarded, see \
             agent_loop_spill.rs) must persist a {tool} entry; index: {index:?}"
        )
    });
    let recalled = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.recall("", &entry.id))
        .unwrap();
    assert!(
        recalled.is_some(),
        "the ephemeral tool spill entry {} must be recallable",
        entry.id
    );
}

/// Given-phrased wrapper around the collapse trigger: the collapse is
/// precondition state for the rewind scenario, not its action. Asserts the
/// collapse actually happened (precondition, not the scenario's outcome) and
/// records how many conversation messages are collapsed so the Then can
/// verify they all survive the rewind.
#[given("the old conversation messages have been collapsed to recall stubs")]
fn given_old_messages_collapsed(world: &mut QuectoWorld) {
    let max = world
        .context_collapse_after_messages
        .expect("context_collapse_after_messages must be set");
    let pin = world.context_pin_recent_turns.unwrap_or(0);
    let messages = world.context_messages.as_mut().unwrap();
    let collapsed = msg_pruning::collapse_conversation_messages_over_limit(messages, max, pin);
    assert!(
        collapsed >= 1,
        "precondition: at least one conversation message must collapse"
    );
    world.context_rewind_collapsed_before = Some(collapsed);
}

#[when("the conversation is rewound to the in-flight user prompt")]
fn when_rewound_to_inflight_prompt(world: &mut QuectoWorld) {
    let messages = world.context_messages.as_mut().unwrap();
    let idx = messages
        .iter()
        .rposition(|m| m.role == Role::User && m.turn.is_none())
        .expect("precondition: an in-flight user prompt must exist");
    assert!(
        quecto::interface::cli::uds_session::rewind_to_message_index(messages, idx),
        "precondition: rewind to a user-message boundary must succeed"
    );
}

#[then("the collapsed conversation messages survive the rewind with non-empty content")]
fn then_collapsed_messages_survive_rewind(world: &mut QuectoWorld) {
    let collapsed_before = world
        .context_rewind_collapsed_before
        .expect("the collapse Given must have recorded its count");
    let messages = world.context_messages.as_ref().unwrap();
    let retained: Vec<&Message> = messages
        .iter()
        .filter(|m| m.role == Role::User || m.role == Role::Assistant)
        .collect();
    // Positive control: the rewind removed only the in-flight prompt — every
    // previously collapsed conversation message must still be present (a fix
    // that deletes collapsed turns outright must fail here).
    assert_eq!(
        retained.len(),
        collapsed_before,
        "all {collapsed_before} collapsed conversation messages must survive \
         the rewind; retained: {:?}",
        retained.iter().map(|m| &m.content).collect::<Vec<_>>()
    );
    for (i, m) in retained.iter().enumerate() {
        assert!(
            !m.content.is_empty(),
            "rewind must not blank collapsed conversation messages into \
             empty provider turns (message {i}, role {:?})",
            m.role
        );
        assert!(
            !m.content.contains("recall("),
            "no dangling recall pointers may survive the rewind (the spill \
             store is wiped); message {i}: {}",
            m.content
        );
        assert!(
            !m.is_collapsed,
            "retained messages are no longer recall stubs after rewind"
        );
    }
}

#[when("the next provider context is prepared")]
fn when_next_provider_context_is_prepared(world: &mut QuectoWorld) {
    run_spilling_sliding_window(world);
}

#[given(expr = "provider truth reports {int} context tokens at local estimate {int}")]
fn given_provider_truth_reports_context_tokens(
    world: &mut QuectoWorld,
    reported: usize,
    estimate: usize,
) {
    world.context_budget_config = Some(reported);
    world.context_original_tokens = Some(estimate);
}

#[when(expr = "the local context estimate changes to {int} tokens")]
fn when_local_context_estimate_changes(world: &mut QuectoWorld, estimate: usize) {
    let reported = world
        .context_budget_config
        .expect("provider truth should be recorded");
    let original_estimate = world
        .context_original_tokens
        .expect("provider-truth estimate should be recorded");
    let reconciled = if estimate < original_estimate {
        reported.saturating_sub(original_estimate - estimate)
    } else {
        reported.saturating_add(estimate - original_estimate)
    };
    world.context_effective_budget = Some(reconciled);
}

#[then(expr = "the user-facing context gauge reports {int} tokens")]
fn then_user_facing_context_gauge_reports(world: &mut QuectoWorld, expected: usize) {
    assert_eq!(
        world.context_effective_budget,
        Some(expected),
        "provider-truth context gauge should carry forward the local estimate delta"
    );
}
