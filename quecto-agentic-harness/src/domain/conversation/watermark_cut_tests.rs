use super::*;
use crate::domain::conversation::UserKind;
use crate::domain::conversation::watermark::{CutInput, CutTrigger, Watermark, plan_cut};
use crate::domain::message::{Role, ToolCall};
use crate::domain::turn_origin::{harness_note, instruction, progress_nudge, prompt};

/// About `tokens` estimated tokens of prose.
fn text(tag: &str, tokens: usize) -> String {
    let mut text = format!("{tag} ");
    while Message::estimate_tokens(&text) < tokens {
        text.push_str("lorem ipsum dolor sit amet ");
    }
    text
}

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "read".to_string(),
        arguments: format!(r#"{{"path":"src/{id}.rs"}}"#),
    }
}

fn result(id: &str, tokens: usize) -> Message {
    let mut message = Message::tool(id.to_string(), text(id, tokens));
    message.tool_name = Some("read".to_string());
    message
}

/// One exchange: a call and its result of `tokens`.
fn exchange(messages: &mut Vec<Message>, id: &str, tokens: usize) {
    messages.push(Message::assistant("", vec![call(id)]));
    messages.push(result(id, tokens));
}

fn trigger(high: usize, low: usize) -> CutTrigger {
    CutTrigger {
        marks: Watermark::new(high, low).unwrap(),
        ceiling: 1_000_000,
        after_last_cut: None,
    }
}

fn plan(messages: &[Message], high: usize, low: usize) -> CutPlan {
    let view = plan_messages(messages);
    plan_cut(&CutInput {
        messages: &view,
        tool_tokens: 0,
        stub_tokens: stub_tokens_bound(),
        trigger: trigger(high, low),
    })
    .expect("a cut is due")
}

/// A session over 20k: the system prompt, the brief, eight 2k exchanges,
/// the latest prompt and its turn so far.
fn session() -> Vec<Message> {
    let mut messages = vec![
        Message::system(text("system", 300)),
        prompt(text("brief", 200)),
    ];
    for n in 0..8 {
        exchange(&mut messages, &format!("old-{n}"), 2_000);
        messages.push(Message::assistant(
            text(&format!("answer-{n}"), 100),
            vec![],
        ));
    }
    messages.push(prompt(text("latest prompt", 150)));
    exchange(&mut messages, "now-0", 2_000);
    messages
}

#[test]
fn each_message_takes_the_planner_role_of_what_it_is() {
    let stub = archive_stub(3, Some("archive"));
    let messages = vec![
        Message::system("system"),
        prompt("typed".to_string()),
        instruction("the completion nudge".to_string()),
        progress_nudge("continue".to_string()),
        harness_note("<subagent_notification>".to_string(), &[]),
        Message::user("feedback"),
        stub,
        Message::assistant("", vec![call("a"), call("b")]),
        result("a", 10),
    ];
    let roles: Vec<PlanRole<'_>> = messages.iter().map(plan_role).collect();
    assert_eq!(
        roles,
        vec![
            PlanRole::System,
            PlanRole::Prompt,
            PlanRole::User,
            PlanRole::User,
            PlanRole::User,
            PlanRole::User,
            PlanRole::ArchiveStub,
            PlanRole::Assistant {
                calls: vec!["a", "b"]
            },
            PlanRole::ToolResult { call: "a" },
        ]
    );
    let view = plan_messages(&messages);
    assert_eq!(view.len(), messages.len());
    assert!(
        view.iter()
            .zip(&messages)
            .all(|(planned, message)| planned.tokens == message.estimated_tokens()),
        "the planner counts each message's own estimate"
    );
}

#[test]
fn a_prompt_is_marked_and_a_harness_message_is_not() {
    assert_eq!(prompt("typed".to_string()).user_kind, UserKind::Prompt);
    assert_eq!(
        instruction("the completion nudge".to_string()).user_kind,
        UserKind::Unmarked
    );
    assert_eq!(
        harness_note("a wake".to_string(), &[]).user_kind,
        UserKind::Unmarked
    );
    assert_eq!(
        progress_nudge("continue".to_string()).user_kind,
        UserKind::Unmarked
    );
}

#[test]
fn a_harness_message_after_the_latest_prompt_never_displaces_it_from_the_head() {
    let mut messages = session();
    let latest = messages.len() - 3;
    assert_eq!(messages[latest].user_kind, UserKind::Prompt);
    // A sub-agent's note and a swarm wake land after the prompt, mid-turn.
    messages.push(harness_note(
        "<subagent_notification>done".to_string(),
        &messages,
    ));
    exchange(&mut messages, "now-1", 2_000);
    messages.push(harness_note("[swarm wake]".to_string(), &messages));
    exchange(&mut messages, "now-2", 2_000);
    let cut = plan(&messages, 20_000, 6_000);
    assert!(cut.kept().contains(&latest), "the latest prompt is pinned");
    let latest_text = messages[latest].content.clone();
    let stub = archive_stub(9, Some("archive"));
    apply_cut(&mut messages, &cut, stub);
    let stub_at = messages
        .iter()
        .position(|m| m.user_kind == UserKind::ArchiveStub)
        .expect("the stub is in");
    assert!(
        messages[..stub_at].iter().any(|m| m.content == latest_text),
        "the latest prompt stays in the head, before the stub"
    );
}

#[test]
fn the_stub_bound_covers_every_stub() {
    let bound = stub_tokens_bound();
    for count in [0, 1, 9, 10, 99, 1_000, 123_456, usize::MAX] {
        for id in [
            None,
            Some("archive"),
            Some("archive:2"),
            Some("archive:18446744073709551615"),
        ] {
            let stub = archive_stub(count, id);
            assert!(
                stub.estimated_tokens() <= bound,
                "{count} {id:?}: {} over {bound}",
                stub.estimated_tokens()
            );
            assert_eq!(stub.role, Role::User);
            assert_eq!(stub.user_kind, UserKind::ArchiveStub);
        }
    }
}

#[test]
fn the_stub_names_its_archive_and_says_when_there_is_none() {
    let archived = archive_stub(12, Some("archive:3"));
    assert!(archived.content.contains("12 earlier messages"));
    assert!(archived.content.contains(r#"recall("archive:3")"#));
    let dropped = archive_stub(12, None);
    assert!(dropped.content.contains("dropped"), "{}", dropped.content);
    assert!(!dropped.content.contains("archive:"));
}

#[test]
fn a_cut_keeps_the_head_byte_identical_and_puts_the_stub_after_it() {
    let mut messages = session();
    let before: Vec<Message> = messages.clone();
    let cut = plan(&messages, 20_000, 6_000);
    let stub = archive_stub(1, Some("archive"));
    let stub_text = stub.content.clone();
    let archived = apply_cut(&mut messages, &cut, stub);
    let slot = cut.stub_slot();
    for (kept, &index) in messages[..slot].iter().zip(cut.kept()) {
        assert_eq!(kept.id(), before[index].id(), "the head keeps its messages");
        assert_eq!(kept.content, before[index].content, "byte-identical");
    }
    assert_eq!(
        messages[slot].content, stub_text,
        "the stub follows the head"
    );
    for (kept, &index) in messages[slot + 1..].iter().zip(&cut.kept()[slot..]) {
        assert_eq!(kept.id(), before[index].id(), "the tail is kept whole");
    }
    assert_eq!(messages.len(), cut.kept().len() + 1);
    let archived_ids: Vec<_> = archived.iter().map(Message::id).collect();
    let planned: Vec<_> = cut
        .archived()
        .iter()
        .flat_map(Clone::clone)
        .map(|i| before[i].id())
        .collect();
    assert_eq!(archived_ids, planned, "the archived messages, in order");
}

#[test]
fn the_index_lists_the_previous_stub_first_then_each_message_with_its_recall_id() {
    let mut brief = prompt(text("brief", 50));
    brief.spill_id = Some("turn0:msg:user".to_string());
    let previous = archive_stub(4, Some("archive"));
    let mut answer = Message::assistant("the first answer", vec![call("c1")]);
    answer.spill_id = Some("turn1:msg:assistant".to_string());
    let mut read = result("c1", 40);
    read.spill_id = Some("turn1:read:0".to_string());
    let unretained = Message::assistant("never spilled, kept in full", vec![]);
    let index = archive_index(&[&brief, &previous, &answer, &read, &unretained]);
    let first_line = index.lines().next().unwrap_or_default();
    assert!(
        first_line.contains(r#"recall("archive")"#),
        "the previous stub comes first: {index}"
    );
    let at = |needle: &str| {
        index
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} in {index}"))
    };
    assert!(at(r#"recall("turn0:msg:user")"#) < at(r#"recall("turn1:msg:assistant")"#));
    assert!(at(r#"recall("turn1:msg:assistant")"#) < at(r#"recall("turn1:read:0")"#));
    assert!(index.contains("read("), "an assistant line names its calls");
    assert!(
        index.contains("never spilled, kept in full"),
        "an unretained message is kept in full"
    );
}

#[test]
fn archived_lists_what_the_plan_archives_in_order() {
    let messages = session();
    let cut = plan(&messages, 20_000, 6_000);
    let listed: Vec<_> = archived(&messages, &cut).iter().map(|m| m.id()).collect();
    let planned: Vec<_> = cut
        .archived()
        .iter()
        .flat_map(Clone::clone)
        .map(|i| messages[i].id())
        .collect();
    assert!(!listed.is_empty());
    assert_eq!(listed, planned);
}
