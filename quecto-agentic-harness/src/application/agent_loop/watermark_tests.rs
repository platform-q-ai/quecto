//! #2403: the watermark context's pass on the agent loop: no mid-history
//! edit, one cut at the high mark with the head byte-identical, the
//! archive and its recall, the chained archive, the storm guard and its
//! reset, an interrupted pass, and the default mode unchanged.

use super::ctx_mgmt_tests::MemSpillStore;
use crate::application::agent_loop::tests::{MockProvider, MockRegistry};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::sessions::dto::retained_context::{RecallOutcome, RecallQuery};
use crate::application::sessions::ports::ContextSpillStore;
use crate::application::sessions::use_cases::RecallContext;
use crate::domain::conversation::watermark::Watermark;
use crate::domain::conversation::{ContextMode, UserKind};
use crate::domain::large_result_collapse::LargeResultCollapse;
use crate::domain::message::{Message, ToolCall};
use crate::domain::session::SpillEntry;
use crate::domain::session_identity::{SessionIdentity, SpillId};
use crate::domain::turn_origin::prompt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

const HIGH: usize = 20_000;
const LOW: usize = 6_000;
const SESSION: &str = "watermark-test";

fn watermark() -> ContextMode {
    ContextMode::Watermark(Watermark::new(HIGH, LOW).unwrap())
}

/// About `tokens` estimated tokens of prose, tagged.
fn text(tag: &str, tokens: usize) -> String {
    let mut text = format!("{tag} ");
    while Message::estimate_tokens(&text) < tokens {
        text.push_str("lorem ipsum dolor sit amet ");
    }
    text
}

fn total(messages: &[Message]) -> usize {
    messages.iter().map(Message::estimated_tokens).sum()
}

/// An agent whose every mid-history rule fires at once in the default
/// mode: two tool results, three messages, a 500-token result seen once.
fn agent(store: Arc<dyn ContextSpillStore>, mode: Option<ContextMode>) -> AgentLoopImpl {
    agent_under(store, mode, 1_000_000, None)
}

/// [`agent`] under a configured budget and a model window.
fn agent_under(
    store: Arc<dyn ContextSpillStore>,
    mode: Option<ContextMode>,
    max_context_tokens: usize,
    model_context_window: Option<usize>,
) -> AgentLoopImpl {
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(MockProvider::new(vec![])),
        tool_registry: Box::new(MockRegistry::new()),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        retention: Some(crate::composition::retention::context_retention_over(store)),
        session_key: SESSION.to_string(),
        context_collapse_after_tool_calls: 2,
        max_context_tokens,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 1,
        context_collapse_after_messages: 3,
        large_result_collapse: LargeResultCollapse {
            over_tokens: 500,
            after_turns: 1,
        },
        model_context_window,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });
    if let Some(mode) = mode {
        agent.set_context_mode(mode);
    }
    agent
}

struct Rig {
    agent: AgentLoopImpl,
    store: Arc<MemSpillStore>,
    calls: usize,
}

impl Rig {
    fn new(mode: Option<ContextMode>) -> Self {
        let store = Arc::new(MemSpillStore::default());
        Self {
            agent: agent(store.clone(), mode),
            store,
            calls: 0,
        }
    }

    fn watermark() -> Self {
        Self::new(Some(watermark()))
    }

    /// The system prompt and the brief, through a first request's pass
    /// (which puts the spill manifest in).
    async fn opened(&self) -> Vec<Message> {
        let mut messages = vec![
            Message::system(text("system", 300)),
            prompt(text("brief", 200)),
        ];
        self.pass(&mut messages).await;
        messages
    }

    /// One exchange: a call, and its result of `tokens`, spilled at
    /// creation as the loop spills it.
    async fn exchange(&mut self, messages: &mut Vec<Message>, tokens: usize) {
        let id = format!("c{}", self.calls);
        self.calls += 1;
        let mut answer = Message::assistant(
            "",
            vec![ToolCall {
                id: id.clone(),
                name: "read".to_string(),
                arguments: format!(r#"{{"path":"src/{id}.rs"}}"#),
            }],
        );
        answer.turn = Some(1);
        messages.push(answer);
        let mut result = Message::tool(id.clone(), text(&id, tokens));
        result.tool_name = Some("read".to_string());
        result.turn = Some(1);
        self.agent
            .spill_tool_message(&mut result, format!("turn1:read:{id}"))
            .await;
        messages.push(result);
    }

    async fn pass(&self, messages: &mut Vec<Message>) -> usize {
        self.agent.apply_context_pruning(messages, 1, true).await
    }

    async fn recall(&self, id: &str) -> Option<SpillEntry> {
        let recall = RecallContext::new(self.store.clone());
        let query = RecallQuery::parse(id).unwrap();
        match recall
            .recall(&SessionIdentity::from_persisted_key(SESSION), &query)
            .await
            .unwrap()
        {
            RecallOutcome::Entry(entry) => Some(entry),
            RecallOutcome::Index(_) | RecallOutcome::Missing(_) => None,
        }
    }
}

fn stubs(messages: &[Message]) -> Vec<&Message> {
    messages
        .iter()
        .filter(|m| m.user_kind == UserKind::ArchiveStub)
        .collect()
}

fn snapshot(messages: &[Message]) -> Vec<(uuid::Uuid, String)> {
    messages
        .iter()
        .map(|m| (m.id(), m.content.clone()))
        .collect()
}

#[tokio::test]
async fn a_cut_at_the_high_mark_keeps_the_head_byte_identical_and_reaches_the_low_mark() {
    let mut rig = Rig::watermark();
    let mut messages = rig.opened().await;
    let head = snapshot(&messages);
    for _ in 0..10 {
        rig.exchange(&mut messages, 2_000).await;
    }
    assert!(total(&messages) >= HIGH);
    let before = snapshot(&messages);
    rig.pass(&mut messages).await;
    let stubs = stubs(&messages);
    assert_eq!(stubs.len(), 1, "one cut, one stub");
    let tail = snapshot(&messages[head.len() + 1..]);
    assert!(
        before.ends_with(&tail),
        "the kept tail is byte-identical to the newest messages before the cut"
    );
    assert_eq!(
        snapshot(&messages[..head.len()]),
        head,
        "the head is byte-identical"
    );
    assert_eq!(
        messages[head.len()].user_kind,
        UserKind::ArchiveStub,
        "the stub follows the head"
    );
    assert!(total(&messages) <= LOW, "down to L: {}", total(&messages));
}

/// The ceiling wins (#2401): a high mark over the configured budget or
/// the model's room is lowered to it, and the low mark with it in
/// proportion, so the cut comes at the ceiling and frees the same share.
#[tokio::test]
async fn the_ceiling_lowers_both_marks() {
    let marks = ContextMode::Watermark(Watermark::new(100_000, 30_000).unwrap());
    for (budget, window) in [(20_000, None), (1_000_000, Some(24_000))] {
        let mut rig = Rig::watermark();
        rig.agent = agent_under(rig.store.clone(), Some(marks), budget, window);
        let ceiling = rig.agent.context_manager.effective_max_context_tokens();
        assert!(ceiling <= 24_000, "{budget} {window:?}: {ceiling}");
        let mut messages = rig.opened().await;
        while total(&messages) < ceiling {
            rig.exchange(&mut messages, 1_000).await;
        }
        rig.pass(&mut messages).await;
        assert_eq!(stubs(&messages).len(), 1, "{budget} {window:?}: a cut");
        let low = 30_000 * ceiling / 100_000;
        assert!(
            total(&messages) <= low,
            "{budget} {window:?}: down to {low}: {}",
            total(&messages)
        );
    }
}

/// Final review L3: where no cut can bring the request under the ceiling
/// (here the pinned brief alone is over it), watermark mode falls back to
/// the default ladder for that request: it never sends over the ceiling
/// where the default mode would not.
#[tokio::test]
async fn watermark_mode_never_sends_over_the_ceiling_where_the_default_mode_would_not() {
    let marks = ContextMode::Watermark(Watermark::new(100_000, 30_000).unwrap());
    for mode in [Some(marks), None] {
        let mut rig = Rig::new(None);
        rig.agent = agent_under(rig.store.clone(), mode, 20_000, None);
        let mut answer = Message::assistant(text("the first answer", 100), vec![]);
        answer.turn = Some(1);
        let mut messages = vec![
            Message::system(text("system", 300)),
            prompt(text("a brief over the ceiling", 30_000)),
            answer,
            prompt(text("latest", 100)),
        ];
        rig.exchange(&mut messages, 300).await;
        let sent = rig.pass(&mut messages).await;
        assert!(sent <= 20_000, "{mode:?}: {sent} over the 20k ceiling");
    }
}

/// No mid-history edit: below H nothing in the conversation changes in
/// watermark mode, where the same dials collapse earlier results and
/// messages in the default mode.
#[tokio::test]
async fn below_the_high_mark_watermark_mode_edits_nothing_the_default_rules_would() {
    async fn session(rig: &mut Rig) -> Vec<Message> {
        let mut messages = rig.opened().await;
        for n in 0..6 {
            rig.exchange(&mut messages, 800).await;
            let mut answer = Message::assistant(text(&format!("answer {n}"), 100), vec![]);
            answer.turn = Some(1);
            messages.push(answer);
        }
        messages
    }
    let mut rig = Rig::watermark();
    let mut messages = session(&mut rig).await;
    let before = snapshot(&messages);
    assert!(total(&messages) < HIGH);
    rig.pass(&mut messages).await;
    assert_eq!(snapshot(&messages), before, "nothing is edited");

    let mut default = Rig::new(None);
    let mut messages = session(&mut default).await;
    default.pass(&mut messages).await;
    assert!(
        messages.iter().filter(|m| m.is_collapsed).count() > 0,
        "the default rules collapse on the same dials"
    );
}

/// The default mode is unchanged: switching it on explicitly prunes
/// exactly as an agent never switched does, and an agent starts in it.
#[tokio::test]
async fn the_default_mode_prunes_as_before() {
    async fn pruned(mut rig: Rig) -> Vec<(bool, String)> {
        let mut messages = rig.opened().await;
        for n in 0..8 {
            rig.exchange(&mut messages, 900).await;
            let mut answer = Message::assistant(text(&format!("answer {n}"), 120), vec![]);
            answer.turn = Some(1);
            messages.push(answer);
        }
        rig.pass(&mut messages).await;
        messages
            .iter()
            .map(|m| (m.is_collapsed, m.content.clone()))
            .collect()
    }
    let never_switched = Rig::new(None);
    assert_eq!(never_switched.agent.context_mode(), ContextMode::Default);
    let unswitched = pruned(never_switched).await;
    let switched = pruned(Rig::new(Some(ContextMode::Default))).await;
    assert_eq!(unswitched, switched);
    let collapsed = unswitched
        .iter()
        .filter(|(collapsed, _)| *collapsed)
        .count();
    assert_eq!(collapsed, 8, "the dials collapse as they always have");
}

#[tokio::test]
async fn recall_reaches_the_archive_and_each_archived_message() {
    let mut rig = Rig::watermark();
    let mut messages = rig.opened().await;
    for _ in 0..10 {
        rig.exchange(&mut messages, 2_000).await;
    }
    rig.pass(&mut messages).await;
    let stub = stubs(&messages)[0].content.clone();
    assert!(stub.contains(r#"recall("archive")"#), "{stub}");
    let index = rig.recall("archive").await.expect("the archive index");
    assert!(
        index.content.contains(r#"recall("turn1:read:c0")"#),
        "the index lists the oldest result: {}",
        index.content
    );
    assert!(
        !messages.iter().any(|m| m.content.starts_with("c0 ")),
        "the result was archived"
    );
    let archived = rig.recall("turn1:read:c0").await.expect("the result");
    assert_eq!(archived.content, text("c0", 2_000));
}

#[tokio::test]
async fn a_second_cut_chains_its_archive_to_the_first() {
    let mut rig = Rig::watermark();
    let mut messages = rig.opened().await;
    for _ in 0..10 {
        rig.exchange(&mut messages, 2_000).await;
    }
    rig.pass(&mut messages).await;
    for _ in 0..10 {
        rig.exchange(&mut messages, 2_000).await;
    }
    rig.pass(&mut messages).await;
    let stubs = stubs(&messages);
    assert_eq!(stubs.len(), 1, "the previous stub was archived");
    assert!(
        stubs[0].content.contains(r#"recall("archive:2")"#),
        "{}",
        stubs[0].content
    );
    let index = rig.recall("archive:2").await.expect("the second index");
    let first = index.content.lines().next().unwrap_or_default();
    assert!(
        first.contains(r#"recall("archive")"#),
        "the previous stub comes first: {}",
        index.content
    );
}

/// A cut whose newest exchange alone is near H leaves the context near H:
/// the next exchange over H does not cut again until the messages grew by
/// (H - L) / 2. A rewind or a clear takes the stub away, and the baseline
/// with it: the next request over H cuts.
#[tokio::test]
async fn the_storm_guard_holds_until_the_stub_leaves_the_conversation() {
    for leave in ["rewind", "clear"] {
        let mut rig = Rig::watermark();
        let mut messages = rig.opened().await;
        messages.push(prompt(text("latest", 100)));
        for _ in 0..3 {
            rig.exchange(&mut messages, 2_000).await;
        }
        rig.exchange(&mut messages, 17_000).await;
        rig.pass(&mut messages).await;
        let first = stubs(&messages).first().map(|stub| stub.id());
        assert!(first.is_some(), "{leave}: the first cut");
        rig.exchange(&mut messages, 2_500).await;
        assert!(total(&messages) >= HIGH, "{leave}: over H again");
        rig.pass(&mut messages).await;
        assert_eq!(
            stubs(&messages).first().map(|stub| stub.id()),
            first,
            "{leave}: the guard holds the next cut back"
        );
        match leave {
            "rewind" => {
                let latest = messages
                    .iter()
                    .rposition(|m| m.user_kind == UserKind::Prompt)
                    .unwrap();
                assert!(
                    crate::application::sessions::use_cases::rewind_conversation::rewind_to_message_index_for_test(
                        &mut messages,
                        latest,
                    )
                );
            }
            _ => crate::domain::conversation_edit::clear_conversation(&mut messages),
        }
        assert!(stubs(&messages).is_empty(), "{leave}: the stub left");
        messages.push(prompt(text("again", 100)));
        for _ in 0..3 {
            rig.exchange(&mut messages, 2_000).await;
        }
        rig.exchange(&mut messages, 14_500).await;
        assert!(total(&messages) >= HIGH, "{leave}: over H");
        rig.pass(&mut messages).await;
        assert_eq!(
            stubs(&messages).len(),
            1,
            "{leave}: no baseline holds it back"
        );
    }
}

/// Review M2: a rewind to a prompt after the stub keeps the stub, but the
/// archive it names is wiped with the session memory. The stub then names
/// no archive, the baseline is reset, and the next cut's index never
/// points at an archive of the old name.
#[tokio::test]
async fn a_rewind_after_the_stub_leaves_no_dangling_archive_and_resets_the_guard() {
    let mut rig = Rig::watermark();
    let mut messages = rig.opened().await;
    messages.push(prompt(text("latest", 100)));
    for _ in 0..3 {
        rig.exchange(&mut messages, 2_000).await;
    }
    rig.exchange(&mut messages, 17_000).await;
    rig.pass(&mut messages).await;
    assert_eq!(stubs(&messages).len(), 1, "the first cut");
    let target = messages.len();
    messages.push(prompt(text("after the cut", 100)));
    rig.exchange(&mut messages, 500).await;
    assert!(
        crate::application::sessions::use_cases::rewind_conversation::rewind_to_message_index_for_test(
            &mut messages,
            target,
        )
    );
    // The rewind transaction wipes the session memory.
    rig.store
        .clear(&SessionIdentity::from_persisted_key(SESSION))
        .await
        .unwrap();
    let stub = stubs(&messages)[0].content.clone();
    assert!(!stub.contains("recall("), "no dangling recall: {stub}");
    messages.push(prompt(text("again", 100)));
    rig.exchange(&mut messages, 2_500).await;
    rig.exchange(&mut messages, 2_500).await;
    assert!(total(&messages) >= HIGH, "over H");
    rig.pass(&mut messages).await;
    let after = stubs(&messages);
    assert_eq!(after.len(), 1);
    assert!(
        after[0].content.contains(r#"recall("archive")"#),
        "a new cut, no baseline holds it back: {}",
        after[0].content
    );
    let index = rig.recall("archive").await.expect("the new index");
    let first = index.content.lines().next().unwrap_or_default();
    assert!(
        !first.contains(r#"recall("archive")"#),
        "the new index never points at itself: {}",
        index.content
    );
}

/// Review L1: feedback after a reply cut off before anything visible goes
/// in as its own message in watermark mode: the prompt already sent, and
/// pinned, is never edited.
#[tokio::test]
async fn feedback_never_edits_a_sent_prompt_in_watermark_mode() {
    use super::super::agent_loop_errors::{Feedback, append_feedback};
    for (mode, expected) in [
        (watermark(), Feedback::Added),
        (ContextMode::Default, Feedback::Merged),
    ] {
        let mut messages = vec![prompt("the prompt".to_string())];
        let how = append_feedback(&mut messages, "feedback".to_string(), 1, mode);
        assert_eq!(how, expected, "{mode:?}");
        assert_eq!(messages[0].user_kind, UserKind::Prompt);
    }
    let mut messages = vec![prompt("the prompt".to_string())];
    append_feedback(&mut messages, "feedback".to_string(), 1, watermark());
    assert_eq!(
        messages[0].content, "the prompt",
        "the sent prompt is unchanged"
    );
    assert_eq!(messages[1].turn, Some(1), "feedback is inside the turn");
    assert_eq!(messages[1].user_kind, UserKind::Unmarked, "never a prompt");
}

/// A store that never finishes writing an archive.
#[derive(Debug, Default)]
struct StuckArchive(MemSpillStore);

impl ContextSpillStore for StuckArchive {
    fn append(
        &self,
        session_key: &SessionIdentity,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        match entry.id.starts_with("archive") {
            true => Box::pin(std::future::pending()),
            false => self.0.append(session_key, entry),
        }
    }

    fn recall(
        &self,
        session_key: &SessionIdentity,
        id: &SpillId,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<SpillEntry>, crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        self.0.recall(session_key, id)
    }

    fn list_entries(
        &self,
        session_key: &SessionIdentity,
    ) -> crate::application::sessions::ports::SpillIndexList<'_> {
        self.0.list_entries(session_key)
    }

    fn clear(
        &self,
        session_key: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.0.clear(session_key)
    }
}

/// A pass cancelled while it archives (the turn interrupted) leaves no
/// half-applied cut: the conversation is as it was.
#[tokio::test]
async fn an_interrupted_pass_leaves_no_half_applied_cut() {
    let mut rig = Rig::watermark();
    rig.agent = agent(Arc::new(StuckArchive::default()), Some(watermark()));
    let mut messages = rig.opened().await;
    for _ in 0..10 {
        rig.exchange(&mut messages, 2_000).await;
    }
    let before = snapshot(&messages);
    let interrupted = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        rig.pass(&mut messages),
    )
    .await;
    assert!(interrupted.is_err(), "the pass waits on the archive");
    assert_eq!(snapshot(&messages), before, "nothing was cut");
    assert!(stubs(&messages).is_empty());
}
