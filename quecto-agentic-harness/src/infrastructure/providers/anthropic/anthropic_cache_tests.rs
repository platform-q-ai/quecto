//! #2410: Anthropic prompt caching, checked against the cache's rules as
//! the Messages API documents them: a request writes the prefix that ends
//! at each `cache_control` block, and reads the longest prefix it wrote
//! before that ends at a block boundary within the 20 positions that end at
//! a marked block (the marked one counted); at most four blocks are marked.
//! A run of consecutive `tool_use` blocks is one position, and so is a run
//! of consecutive `tool_result` blocks (#2628 review). The prefix is the
//! request's tools, then its system blocks, then each message's content
//! blocks in order; a marker is not part of it, and a string `content` is
//! the one text block it stands for.
use super::*;
use crate::domain::conversation::value_objects::message::ToolCall;
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};

/// The positions the cache looks back over from a marker, the marker's own
/// counted.
const LOOKBACK: usize = 20;
/// The most blocks one request may mark.
const MAX_BREAKPOINTS: usize = 4;

/// The request `messages` are sent as.
fn body(messages: &[Message], is_oauth: bool) -> serde_json::Value {
    let tools = [
        crate::domain::tool_policy::value_objects::tool::ToolDefinition {
            name: "bash".into(),
            description: "run".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        },
    ];
    let request = ChatRequest {
        trace: None,
        admission: None,
        messages,
        tools: &tools,
        model: "claude-sonnet-4-5",
        max_tokens: 100,
        temperature: 0.0,
        session_id: Some("s"),
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    AnthropicProvider::build_request_body(&request, is_oauth).1
}

/// One block of a request's prefix: its canonical text (without its marker),
/// and whether it is marked.
struct Block {
    text: String,
    marked: bool,
    /// The block's `type`: runs of `tool_use` or `tool_result` blocks share
    /// one position.
    kind: String,
}

fn block(scope: &str, value: &serde_json::Value) -> Block {
    let mut value = value.clone();
    let marked = value
        .as_object_mut()
        .and_then(|block| block.remove("cache_control"))
        .is_some();
    Block {
        kind: value["type"].as_str().unwrap_or_default().to_string(),
        text: format!("{scope}:{value}"),
        marked,
    }
}

/// The request's prefix, block by block, in the cache's order.
fn prefix(body: &serde_json::Value) -> Vec<Block> {
    let mut blocks: Vec<Block> = body["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|tool| block("tool", tool))
        .collect();
    match &body["system"] {
        serde_json::Value::String(text) => blocks.push(block(
            "system",
            &serde_json::json!({"type": "text", "text": text}),
        )),
        serde_json::Value::Array(parts) => {
            blocks.extend(parts.iter().map(|part| block("system", part)))
        }
        _ => {}
    }
    for message in body["messages"].as_array().expect("messages") {
        let role = message["role"].as_str().expect("a role");
        match &message["content"] {
            serde_json::Value::String(text) => blocks.push(block(
                role,
                &serde_json::json!({"type": "text", "text": text}),
            )),
            serde_json::Value::Array(parts) => {
                blocks.extend(parts.iter().map(|part| block(role, part)))
            }
            other => panic!("unexpected content {other}"),
        }
    }
    blocks
}

/// The digest of each prefix of `blocks`: `digests[i]` covers blocks `..=i`.
fn digests(blocks: &[Block]) -> Vec<u64> {
    let mut hasher = DefaultHasher::new();
    blocks
        .iter()
        .map(|block| {
            block.text.hash(&mut hasher);
            hasher.finish()
        })
        .collect()
}

/// The position of each block: a block opens a new one unless it continues
/// a run of `tool_use` or `tool_result` blocks.
fn positions(blocks: &[Block]) -> Vec<usize> {
    let mut position = 0;
    blocks
        .iter()
        .enumerate()
        .map(|(at, block)| {
            let runs = matches!(block.kind.as_str(), "tool_use" | "tool_result")
                && at > 0
                && blocks[at - 1].kind == block.kind;
            if at > 0 && !runs {
                position += 1;
            }
            position
        })
        .collect()
}

/// A session's cache, as the API keeps it.
#[derive(Default)]
struct Cache {
    written: HashSet<u64>,
}

/// What one request of a session read and wrote.
struct Sent {
    /// The blocks read from the cache: the prefix up to the cached boundary.
    read: usize,
    /// The block the request's last marker ends at, plus one.
    last_marker: usize,
    blocks: usize,
    markers: usize,
}

impl Cache {
    fn send(&mut self, body: &serde_json::Value) -> Sent {
        let blocks = prefix(body);
        let digests = digests(&blocks);
        let positions = positions(&blocks);
        let markers: Vec<usize> = (0..blocks.len()).filter(|&i| blocks[i].marked).collect();
        let read = markers
            .iter()
            .rev()
            .find_map(|&marker| {
                let earliest = positions[marker].saturating_sub(LOOKBACK - 1);
                (0..=marker)
                    .rev()
                    .take_while(|&boundary| positions[boundary] >= earliest)
                    .find(|&boundary| self.written.contains(&digests[boundary]))
                    .map(|boundary| boundary + 1)
            })
            .unwrap_or(0);
        for &marker in &markers {
            self.written.insert(digests[marker]);
        }
        Sent {
            read,
            last_marker: markers.last().map_or(0, |marker| marker + 1),
            blocks: blocks.len(),
            markers: markers.len(),
        }
    }
}

fn calls(count: usize) -> Vec<ToolCall> {
    (0..count)
        .map(|i| ToolCall {
            id: format!("c{i}"),
            name: "bash".into(),
            arguments: format!(r#"{{"command":"ls {i}"}}"#),
        })
        .collect()
}

/// One assistant turn that calls `count` tools, and their results.
fn tool_turn(messages: &mut Vec<Message>, count: usize) {
    let calls = calls(count);
    messages.push(Message::assistant("", calls.clone()));
    for call in calls {
        messages.push(Message::tool(&call.id, format!("output of {}", call.id)));
    }
}

/// Each request after the first reads the whole conversation the request
/// before it sent from the cache: everything up to that request's last
/// marker.
fn assert_each_request_reads_the_last(steps: &[Vec<Message>], is_oauth: bool) {
    let mut cache = Cache::default();
    let mut previous: Option<Sent> = None;
    for (step, messages) in steps.iter().enumerate() {
        let sent = cache.send(&body(messages, is_oauth));
        assert!(
            sent.markers <= MAX_BREAKPOINTS,
            "step {step}: {} markers",
            sent.markers
        );
        assert_eq!(
            sent.last_marker, sent.blocks,
            "step {step}: the last block is marked"
        );
        if let Some(previous) = previous {
            assert!(
                sent.read >= previous.last_marker,
                "step {step}: read {} of the {} blocks the request before wrote",
                sent.read,
                previous.last_marker
            );
        }
        previous = Some(sent);
    }
}

/// #2410: a reply that calls many tools at once adds more blocks than the
/// cache looks back over from the new last marker; the request after it
/// still reads the whole conversation before it.
#[test]
fn a_wide_tool_batch_still_reads_the_whole_previous_conversation() {
    for is_oauth in [false, true] {
        let mut messages = vec![Message::system("sys"), Message::user("go")];
        let mut steps = vec![messages.clone()];
        tool_turn(&mut messages, 15);
        steps.push(messages.clone());
        tool_turn(&mut messages, 1);
        steps.push(messages.clone());
        tool_turn(&mut messages, 12);
        steps.push(messages.clone());
        assert_each_request_reads_the_last(&steps, is_oauth);
    }
}

/// #2410: a steer after a tool batch (a user message after the results)
/// still reads the whole conversation the request before sent.
#[test]
fn a_steer_after_a_tool_batch_still_reads_the_previous_conversation() {
    let mut messages = vec![Message::system("sys"), Message::user("go")];
    let mut steps = vec![messages.clone()];
    tool_turn(&mut messages, 3);
    steps.push(messages.clone());
    tool_turn(&mut messages, 14);
    messages.push(Message::user("stop and summarise"));
    steps.push(messages.clone());
    tool_turn(&mut messages, 2);
    steps.push(messages.clone());
    assert_each_request_reads_the_last(&steps, false);
}

/// #2410: a watermark cut (#2403) as the harness builds it: the pinned
/// brief and the latest prompt, the archive stub, then the kept tail, which
/// opens on an assistant turn. The request at the cut reads only what it
/// shares with the requests before — the tools, the system prompt and the
/// brief — so the cut is one cache write; the request after it reads the
/// whole cut conversation again.
#[test]
fn a_watermark_cut_misses_once_then_reads_again() {
    use crate::domain::conversation::services::turn_origin::prompt;
    use crate::domain::conversation::services::watermark_cut::archive_stub;
    let mut cache = Cache::default();
    let brief = prompt("the brief".to_string());
    let latest = prompt("the latest prompt".to_string());
    let mut messages = vec![Message::system("sys"), brief.clone()];
    cache.send(&body(&messages, false));
    tool_turn(&mut messages, 4);
    messages.push(latest.clone());
    cache.send(&body(&messages, false));
    tool_turn(&mut messages, 2);
    let before_cut = cache.send(&body(&messages, false));
    assert!(
        before_cut.read > 3,
        "the conversation was cached before the cut"
    );
    let mut cut = vec![
        Message::system("sys"),
        brief,
        latest,
        archive_stub(9, Some("archive")),
    ];
    tool_turn(&mut cut, 2);
    let at_cut = cache.send(&body(&cut, false));
    assert_eq!(
        at_cut.read, 3,
        "the cut reads the tools, the system prompt and the brief only"
    );
    assert_eq!(at_cut.last_marker, at_cut.blocks);
    assert!(at_cut.markers <= MAX_BREAKPOINTS);
    tool_turn(&mut cut, 3);
    let after_cut = cache.send(&body(&cut, false));
    assert!(
        after_cut.read >= at_cut.last_marker,
        "after the cut: read {} of {}",
        after_cut.read,
        at_cut.last_marker
    );
}

/// #2628 review: a turn that adds more than 20 positions that do not
/// collapse — here a reply, then three queued steers each carrying the
/// eight images a prompt may (1 + 3 x 9 = 28 positions) — outruns the
/// look-back from the new last marker, so only the second breakpoint, at
/// the message the request before ended at, reads the conversation back.
#[test]
fn a_turn_of_more_than_twenty_positions_reads_back_through_the_second_breakpoint() {
    use crate::domain::conversation::value_objects::message::UserImageBlock;
    let mut messages = vec![Message::system("sys"), Message::user("go")];
    let mut steps = vec![messages.clone()];
    messages.push(Message::assistant("look at these", vec![]));
    for steer in 0..3 {
        let mut shots = Message::user(format!("and these, set {steer}"));
        for at in 0..8 {
            shots
                .user_image_blocks
                .push(UserImageBlock::unchecked_for_tests(
                    quecto_image::ImageMime::Png,
                    format!("image{steer}-{at}"),
                ));
        }
        messages.push(shots);
    }
    steps.push(messages.clone());
    assert_each_request_reads_the_last(&steps, false);
}

/// #2628 review: the second breakpoint marks only a user message — never an
/// assistant reply before the last one, however the conversation runs.
#[test]
fn only_user_messages_are_marked() {
    let mut api_messages = vec![
        serde_json::json!({"role": "user", "content": "go"}),
        serde_json::json!({"role": "assistant", "content": "one"}),
        serde_json::json!({"role": "assistant", "content": "two"}),
        serde_json::json!({"role": "user", "content": "next"}),
    ];
    AnthropicProvider::apply_cache_control_to_last_user_message(&mut api_messages);
    let marked: Vec<&str> = api_messages
        .iter()
        .filter(|message| message.to_string().contains("cache_control"))
        .map(|message| message["role"].as_str().unwrap())
        .collect();
    assert_eq!(marked, ["user"], "{api_messages:?}");
    assert!(api_messages[3].to_string().contains("cache_control"));
}

/// #2410 acceptance: in a long tool-heavy session (batches of 1 to 15 tool
/// calls, an occasional steer), each request reads everything but what the
/// turn before it appended, so the steady-state share of the prompt read
/// from the cache is at least 97% by the token estimate once the
/// conversation is large, as with Codex.
#[test]
fn a_long_tool_session_reads_at_least_97_percent_of_its_prompt_from_the_cache() {
    use crate::domain::inference::value_objects::token_estimate::estimate_tokens;
    let mut cache = Cache::default();
    let mut messages = vec![
        Message::system("rules ".repeat(400)),
        Message::user("start"),
    ];
    let (mut read, mut total) = (0usize, 0usize);
    for turn in 0..60 {
        let batch = 1 + (turn * 7) % 15;
        let calls = calls(batch);
        messages.push(Message::assistant("", calls.clone()));
        for call in calls {
            messages.push(Message::tool(&call.id, "output line\n".repeat(60)));
        }
        if turn % 9 == 4 {
            messages.push(Message::user("also check the tests"));
        }
        let body = body(&messages, false);
        let blocks = prefix(&body);
        let sent = cache.send(&body);
        assert!(
            sent.markers <= MAX_BREAKPOINTS,
            "turn {turn}: {} markers",
            sent.markers
        );
        let tokens = |blocks: &[Block]| -> usize {
            blocks
                .iter()
                .map(|block| estimate_tokens(&block.text))
                .sum()
        };
        if turn >= 20 {
            read += tokens(&blocks[..sent.read]);
            total += tokens(&blocks);
        }
    }
    let share = read as f64 / total as f64;
    assert!(share >= 0.97, "steady-state cache read share {share:.4}");
}
