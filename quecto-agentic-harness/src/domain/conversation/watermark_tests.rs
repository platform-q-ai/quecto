use super::*;
use crate::domain::message::ToolCall;

/// About `tokens` estimated tokens of prose.
fn prose(tokens: usize) -> String {
    "word ".repeat(tokens * 4 / 5)
}

/// One exchange: an assistant call and its result of `tokens`.
fn exchange(n: usize, tokens: usize) -> Vec<Message> {
    let id = format!("call-{n}");
    let call = Message::assistant(
        "",
        vec![ToolCall {
            id: id.clone(),
            name: "read".into(),
            arguments: format!(r#"{{"n":{n}}}"#),
        }],
    );
    vec![call, Message::tool(id, prose(tokens))]
}

fn conversation(exchanges: usize, tokens: usize) -> Vec<Message> {
    let mut messages = vec![Message::system("sys"), Message::user("the brief")];
    for n in 0..exchanges {
        messages.extend(exchange(n, tokens));
    }
    messages
}

fn total(messages: &[Message]) -> usize {
    messages.iter().map(Message::estimated_tokens).sum()
}

#[test]
fn marks_need_low_under_high() {
    assert!(ContextWatermark::new(100, 30).is_some());
    assert!(ContextWatermark::new(30, 30).is_none());
    assert!(ContextWatermark::new(100, 0).is_none());
}

#[test]
fn no_cut_under_the_high_mark() {
    let messages = conversation(10, 1_000);
    let high = total(&messages) + 1;
    assert_eq!(plan_cut(&messages, high, high / 3), None);
}

#[test]
fn a_cut_keeps_the_head_the_stub_and_whole_recent_exchanges_under_low() {
    let mut messages = conversation(40, 1_000);
    let high = total(&messages);
    let low = high / 4;
    let cut = plan_cut(&messages, high, low).expect("at the high mark");
    assert_eq!(cut.head_end, 2, "system prompt and the brief");
    assert!(!cut.over_low);
    let newest = messages.last().unwrap().content.clone();
    let archived = apply_cut(
        &mut messages,
        cut,
        archive_stub(cut.tail_start - 2, Some("archive")),
    );

    assert_eq!(messages[0].content, "sys");
    assert_eq!(messages[1].content, "the brief");
    assert!(is_archive_stub(&messages[2]));
    assert!(messages[2].content.contains("recall(\"archive\")"));
    assert_eq!(
        messages[3].role,
        Role::Assistant,
        "the tail opens an exchange"
    );
    assert_eq!(messages.last().unwrap().content, newest);
    assert!(total(&messages) <= low, "{} > {low}", total(&messages));
    assert!(tail_is_whole(&messages[3..]));
    assert_eq!(archived.len() + messages.len(), 2 + 80 + 1);
    // As many exchanges as fit: one more would pass the low mark.
    let one_more = archived[archived.len() - 2..]
        .iter()
        .map(Message::estimated_tokens)
        .sum::<usize>();
    assert!(total(&messages) + one_more > low - STUB_RESERVE_TOKENS);
}

#[test]
fn a_second_cut_archives_the_previous_stub_and_keeps_one_stub() {
    let mut messages = conversation(40, 1_000);
    let high = total(&messages);
    let low = high / 4;
    let cut = plan_cut(&messages, high, low).unwrap();
    apply_cut(&mut messages, cut, archive_stub(1, Some("archive")));
    for n in 40..80 {
        messages.extend(exchange(n, 1_000));
    }
    let cut = plan_cut(&messages, high, low).expect("at the high mark again");
    assert_eq!(cut.head_end, 2);
    let archived = apply_cut(&mut messages, cut, archive_stub(9, Some("archive:2")));
    assert!(
        is_archive_stub(&archived[0]),
        "the old stub is archived first"
    );
    assert_eq!(messages.iter().filter(|m| is_archive_stub(m)).count(), 1);
    assert!(messages[2].content.contains("archive:2"));
}

#[test]
fn an_exchange_over_the_low_mark_is_kept_whole() {
    let mut messages = conversation(3, 1_000);
    messages.extend(exchange(9, 50_000));
    let cut = plan_cut(&messages, 10_000, 5_000).unwrap();
    assert!(cut.over_low);
    assert_eq!(cut.tail_start, messages.len() - 2, "the newest exchange");
}

#[test]
fn a_cut_can_fall_before_a_user_message() {
    let mut messages = conversation(10, 1_000);
    messages.push(Message::assistant("done", vec![]));
    messages.push(Message::user("next prompt"));
    let cut = plan_cut(&messages, 1, 1).unwrap();
    assert_eq!(cut.tail_start, messages.len() - 1);
}

#[test]
fn no_head_no_cut() {
    let messages = vec![Message::system("sys"), Message::assistant("hi", vec![])];
    assert_eq!(plan_cut(&messages, 0, 0), None);
}

#[test]
fn the_index_names_recall_ids_and_carries_unspilled_content() {
    let mut spilled = Message::user("spilled prompt");
    spilled.spill_id = Some("turn1:msg:user".into());
    let unspilled = Message::assistant("kept text", vec![]);
    let index = archive_index(&[spilled, unspilled]);
    assert!(index.contains("recall(\"turn1:msg:user\")"));
    assert!(index.contains("kept text"));
}
