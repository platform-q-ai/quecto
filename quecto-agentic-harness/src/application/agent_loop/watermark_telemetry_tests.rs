//! #2404: the watermark context's event-log records. A cut writes one
//! `context_cut` record (never a `context_pruned` one); a cut that is due
//! but not made writes a `context_cut_skipped` record naming why; the
//! emergency ladder writes a `context_pruned` record marked
//! `watermark_fallback`. Every record holds counts, ids and kinds only.

use super::ctx_mgmt_tests::CapturingAuditSink;
use super::watermark_tests::{HIGH, LOW, Rig, agent_under, default_agent, stubs, text, total};
use crate::application::audit::ports::AuditSink;
use crate::domain::audit::{AuditEvent, ContextCutRecord, ContextCutSkippedRecord, CutSkipReason};
use crate::domain::conversation::watermark::{Fill, Watermark};
use crate::domain::message::Message;
use crate::domain::turn_origin::prompt;
use std::sync::Arc;

/// `rig` writing its records to a capturing sink.
fn audited(mut rig: Rig) -> (Rig, Arc<CapturingAuditSink>) {
    let sink = Arc::new(CapturingAuditSink::default());
    rig.agent
        .set_audit_log(Some(sink.clone() as Arc<dyn AuditSink>));
    (rig, sink)
}

fn events(sink: &CapturingAuditSink) -> Vec<AuditEvent> {
    sink.events.lock().unwrap().clone()
}

fn cuts(sink: &CapturingAuditSink) -> Vec<ContextCutRecord> {
    let cut = |event: AuditEvent| match event {
        AuditEvent::ContextCut(record) => Some(record),
        _ => None,
    };
    events(sink).into_iter().filter_map(cut).collect()
}

fn skips(sink: &CapturingAuditSink) -> Vec<ContextCutSkippedRecord> {
    let skip = |event: AuditEvent| match event {
        AuditEvent::ContextCutSkipped(record) => Some(record),
        _ => None,
    };
    events(sink).into_iter().filter_map(skip).collect()
}

/// The `watermark_fallback` flag of every `context_pruned` record.
fn prunes(sink: &CapturingAuditSink) -> Vec<bool> {
    let pruned = |event: AuditEvent| match event {
        AuditEvent::ContextPruned {
            watermark_fallback, ..
        } => Some(watermark_fallback),
        _ => None,
    };
    events(sink).into_iter().filter_map(pruned).collect()
}

/// The brief, then `exchanges` of the given result sizes.
async fn session(rig: &mut Rig, exchanges: &[usize]) -> Vec<Message> {
    let mut messages = rig.opened().await;
    for &tokens in exchanges {
        rig.exchange(&mut messages, tokens).await;
    }
    messages
}

#[tokio::test]
async fn a_cut_writes_one_context_cut_record_and_no_context_pruned_record() {
    let (mut rig, sink) = audited(Rig::watermark());
    let mut messages = session(&mut rig, &[2_000; 10]).await;
    let before = messages.len();
    let tokens_before = total(&messages);
    assert!(tokens_before >= HIGH);
    let sent = rig.pass(&mut messages).await;
    assert_eq!(stubs(&messages).len(), 1, "one cut");
    let records = cuts(&sink);
    assert_eq!(records.len(), 1, "one context_cut record: {records:?}");
    assert_eq!(
        prunes(&sink),
        Vec::<bool>::new(),
        "no context_pruned record"
    );
    assert!(skips(&sink).is_empty(), "a cut made is not skipped");
    let record = &records[0];
    let kept = messages.len() - 1;
    assert_eq!(record.messages_kept, kept);
    assert_eq!(record.messages_archived, before - kept);
    assert!(record.tokens_before >= tokens_before, "{record:?}");
    assert_eq!(
        record.tokens_after, sent,
        "the request's size after the cut"
    );
    assert!(record.tokens_after <= LOW, "{record:?}");
    assert_eq!(record.marks.high_tokens, HIGH);
    assert_eq!(record.marks.low_tokens, LOW);
    assert!(!record.marks.ceiling_lowered_marks);
    assert_eq!(record.marks.estimate_scale_permille, 1000);
    assert_eq!(record.fill, Fill::WithinLow);
    let archive = record.archive_id.as_deref().expect("an archive id");
    assert!(
        rig.recall(archive).await.is_some(),
        "the record's archive id is the one recall reaches"
    );
}

/// The record holds counts, ids and kinds only: none of the archived or
/// kept messages' text, no tool argument, nothing of the stub's text.
#[tokio::test]
async fn the_cut_records_carry_no_content() {
    let (mut rig, sink) = audited(Rig::watermark());
    let mut messages = session(&mut rig, &[2_000; 10]).await;
    rig.pass(&mut messages).await;
    let mut more = session(&mut rig, &[]).await;
    rig.exchange(&mut more, 20_000).await;
    rig.pass(&mut more).await;
    let written: Vec<String> = events(&sink)
        .iter()
        .map(|event| serde_json::to_string(event).unwrap())
        .collect();
    assert!(written.iter().any(|line| line.contains("\"context_cut\"")));
    assert!(
        written
            .iter()
            .any(|line| line.contains("context_cut_skipped"))
    );
    for line in &written {
        for content in ["lorem", "brief", "system ", "src/c", "c0 ", "recall("] {
            assert!(!line.contains(content), "{content:?} in {line}");
        }
    }
}

/// The ceiling wins (#2401) and the record says so: the marks in force
/// are the lowered ones.
#[tokio::test]
async fn a_cut_under_a_lower_ceiling_records_the_lowered_marks() {
    let marks = Watermark::new(100_000, 30_000).unwrap();
    let (mut rig, sink) = audited(Rig::watermark());
    rig.agent = agent_under(rig.store.clone(), Some(marks), 20_000, None);
    let sink_again = sink.clone() as Arc<dyn AuditSink>;
    rig.agent.set_audit_log(Some(sink_again));
    let ceiling = rig.agent.context_manager.effective_max_context_tokens();
    let mut messages = rig.opened().await;
    while total(&messages) < ceiling {
        rig.exchange(&mut messages, 1_000).await;
    }
    rig.pass(&mut messages).await;
    let records = cuts(&sink);
    assert_eq!(records.len(), 1, "{records:?}");
    let marks = records[0].marks;
    assert!(marks.ceiling_lowered_marks, "{marks:?}");
    assert_eq!(marks.ceiling_estimate_tokens, ceiling);
    assert_eq!(marks.high_tokens, ceiling);
    assert_eq!(marks.low_tokens, 30_000 * ceiling / 100_000);
}

/// The newest exchange is kept whole even when it alone takes the kept
/// set over L; the record says so.
#[tokio::test]
async fn a_cut_whose_newest_exchange_is_over_the_low_mark_says_so() {
    let (mut rig, sink) = audited(Rig::watermark());
    let mut exchanges = vec![2_000; 10];
    exchanges.push(8_000);
    let mut messages = session(&mut rig, &exchanges).await;
    rig.pass(&mut messages).await;
    let records = cuts(&sink);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].fill, Fill::NewestExchangeOverLow);
    assert!(records[0].tokens_after > LOW, "{:?}", records[0]);
}

/// A cut that is due but would save under a tenth of H is not made, and
/// the record says why, with what it would have saved.
#[tokio::test]
async fn a_due_cut_that_saves_too_little_is_recorded_as_skipped() {
    let (mut rig, sink) = audited(Rig::watermark());
    let mut messages = session(&mut rig, &[500, 20_000]).await;
    let size = total(&messages);
    assert!(size >= HIGH);
    rig.pass(&mut messages).await;
    assert!(stubs(&messages).is_empty(), "no cut");
    assert!(cuts(&sink).is_empty());
    let records = skips(&sink);
    assert_eq!(records.len(), 1, "{records:?}");
    let record = &records[0];
    let CutSkipReason::SavingTooSmall {
        saving_tokens: saving,
        needed_tokens,
    } = record.reason
    else {
        panic!("{record:?}");
    };
    assert_eq!(needed_tokens, HIGH / 10);
    assert!(saving > 0 && saving < HIGH / 10, "{record:?}");
    assert!(record.tokens >= size, "{record:?}");
    assert_eq!(record.marks.high_tokens, HIGH);
}

/// A due cut with no exchange boundary to cut at, or nothing to archive,
/// is recorded as skipped with that reason, and nothing is cut.
#[tokio::test]
async fn a_due_cut_with_nowhere_to_cut_is_recorded_as_skipped() {
    let (mut rig, sink) = audited(Rig::watermark());
    let mut messages = session(&mut rig, &[21_000]).await;
    rig.pass(&mut messages).await;
    let reasons: Vec<CutSkipReason> = skips(&sink).iter().map(|r| r.reason).collect();
    assert_eq!(reasons, [CutSkipReason::NoBoundary]);

    let (rig, sink) = audited(Rig::watermark());
    let mut messages = vec![
        Message::system(text("system", 300)),
        prompt(text("a brief over the high mark", 21_000)),
    ];
    rig.pass(&mut messages).await;
    let reasons: Vec<CutSkipReason> = skips(&sink).iter().map(|r| r.reason).collect();
    assert_eq!(reasons, [CutSkipReason::NothingArchivable]);
    assert!(cuts(&sink).is_empty());
}

/// Below H nothing is due: no record at all.
#[tokio::test]
async fn below_the_high_mark_no_cut_record_is_written() {
    let (mut rig, sink) = audited(Rig::watermark());
    let mut messages = session(&mut rig, &[2_000; 3]).await;
    rig.pass(&mut messages).await;
    assert!(events(&sink).is_empty(), "{:?}", events(&sink));
}

/// The emergency ladder (final review L3 of #2403) writes a
/// `context_pruned` record marked `watermark_fallback`.
#[tokio::test]
async fn the_ladder_fallback_is_a_context_pruned_record_marked_as_the_fallback() {
    let marks = Watermark::new(100_000, 30_000).unwrap();
    let (mut rig, sink) = audited(Rig::new(None));
    rig.agent = agent_under(rig.store.clone(), Some(marks), 20_000, None);
    rig.agent
        .set_audit_log(Some(sink.clone() as Arc<dyn AuditSink>));
    let mut answer = Message::assistant(text("the first answer", 100), vec![]);
    answer.turn = Some(1);
    let mut messages = vec![
        Message::system(text("system", 300)),
        prompt(text("a brief over the ceiling", 30_000)),
        answer,
        prompt(text("latest", 100)),
    ];
    rig.exchange(&mut messages, 300).await;
    rig.pass(&mut messages).await;
    assert_eq!(prunes(&sink), [true], "{:?}", events(&sink));
}

/// Review M1: a cut that leaves the request over the ceiling, so the
/// emergency ladder runs too, counts each step once: the ladder's
/// `context_pruned` is measured from the size the cut left, not from the
/// size before the cut.
#[tokio::test]
async fn a_cut_then_the_ladder_counts_each_once() {
    let marks = Watermark::new(100_000, 30_000).unwrap();
    let (mut rig, sink) = audited(Rig::new(None));
    rig.agent = agent_under(rig.store.clone(), Some(marks), 20_000, None);
    rig.agent
        .set_audit_log(Some(sink.clone() as Arc<dyn AuditSink>));
    let mut messages = vec![
        Message::system(text("system", 300)),
        prompt(text("a brief that fills most of the ceiling", 15_000)),
    ];
    for tokens in [1_000, 1_000, 1_000, 6_000] {
        rig.exchange(&mut messages, tokens).await;
    }
    rig.pass(&mut messages).await;
    let cuts = cuts(&sink);
    assert_eq!(cuts.len(), 1, "{:?}", events(&sink));
    let pruned: Vec<(usize, usize, bool)> = events(&sink)
        .into_iter()
        .filter_map(|event| match event {
            AuditEvent::ContextPruned {
                tokens_before,
                tokens_after,
                watermark_fallback,
                ..
            } => Some((tokens_before, tokens_after, watermark_fallback)),
            _ => None,
        })
        .collect();
    assert_eq!(pruned.len(), 1, "{pruned:?}");
    let (before, after, fallback) = pruned[0];
    assert!(fallback, "the ladder's record is marked");
    assert_eq!(
        before, cuts[0].tokens_after,
        "the ladder starts where the cut left the request"
    );
    assert!(after <= before, "{pruned:?}");
}

/// #2413 carry-over: a cut whose kept set is still over the ceiling, where
/// the ladder really edits: the brief is no longer in the current prompt's
/// region, so the ladder stubs it. Each step is counted once: the ladder's
/// `context_pruned` starts where the cut left the request.
#[tokio::test]
async fn a_cut_then_a_ladder_that_stubs_counts_each_once() {
    let marks = Watermark::new(100_000, 30_000).unwrap();
    let (mut rig, sink) = audited(Rig::new(None));
    rig.agent = agent_under(rig.store.clone(), Some(marks), 20_000, None);
    rig.agent
        .set_audit_log(Some(sink.clone() as Arc<dyn AuditSink>));
    let brief = prompt(text("a brief that fills most of the ceiling", 12_000));
    let brief_id = brief.id();
    let mut answer = Message::assistant(text("the first answer", 100), vec![]);
    answer.turn = Some(1);
    let mut messages = vec![
        Message::system(text("system", 300)),
        brief,
        answer,
        prompt(text("latest", 100)),
    ];
    for (turn, tokens) in [(1, 1_500), (2, 1_500), (3, 1_500), (4, 1_500), (5, 8_000)] {
        rig.exchange(&mut messages, tokens).await;
        let len = messages.len();
        for message in &mut messages[len - 2..] {
            message.turn = Some(turn);
        }
    }
    rig.pass(&mut messages).await;
    let cuts = cuts(&sink);
    assert_eq!(cuts.len(), 1, "{:?}", events(&sink));
    assert!(
        cuts[0].tokens_after > 20_000,
        "the cut left it over: {cuts:?}"
    );
    let pruned: Vec<(usize, usize, usize, bool)> = events(&sink)
        .into_iter()
        .filter_map(|event| match event {
            AuditEvent::ContextPruned {
                tokens_before,
                tokens_after,
                ladder_stubbed,
                watermark_fallback,
                ..
            } => Some((
                tokens_before,
                tokens_after,
                ladder_stubbed,
                watermark_fallback,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(pruned.len(), 1, "{pruned:?}");
    let (before, after, stubbed, fallback) = pruned[0];
    assert!(fallback, "the ladder's record is marked");
    assert!(stubbed >= 1, "the ladder stubbed: {pruned:?}");
    assert_eq!(
        before, cuts[0].tokens_after,
        "it starts where the cut left it"
    );
    assert!(after < before && after <= 20_000, "{pruned:?}");
    let brief = messages.iter().find(|m| m.id() == brief_id).unwrap();
    assert!(brief.is_collapsed, "the ladder stubbed the brief");
}

/// Review L4 and L5: a skip's reason carries its own numbers (adjacently
/// tagged), and the cut's ceiling names its unit (estimate tokens), apart
/// from `context_pruned.ceiling_tokens` (provider tokens).
#[tokio::test]
async fn the_records_name_their_reason_and_units_on_the_wire() {
    let (mut rig, sink) = audited(Rig::watermark());
    let mut messages = session(&mut rig, &[500, 20_000]).await;
    rig.pass(&mut messages).await;
    let mut more = session(&mut rig, &[]).await;
    rig.exchange(&mut more, 21_000).await;
    rig.pass(&mut more).await;
    let mut cut = session(&mut rig, &[2_000; 10]).await;
    rig.pass(&mut cut).await;
    let written: Vec<serde_json::Value> = events(&sink)
        .iter()
        .map(|event| serde_json::to_value(event).unwrap())
        .collect();
    let skipped: Vec<&serde_json::Value> = written
        .iter()
        .filter(|json| json["event"] == "context_cut_skipped")
        .collect();
    assert_eq!(skipped.len(), 2, "{written:?}");
    let saving = &skipped[0]["reason"];
    assert_eq!(saving["kind"], "saving_too_small", "{saving}");
    assert_eq!(saving["detail"]["needed_tokens"], HIGH / 10, "{saving}");
    assert!(saving["detail"]["saving_tokens"].is_u64(), "{saving}");
    assert!(skipped[0].get("saving_tokens").is_none(), "{}", skipped[0]);
    assert_eq!(
        skipped[1]["reason"],
        serde_json::json!({"kind": "no_boundary"})
    );
    let made = written
        .iter()
        .find(|json| json["event"] == "context_cut")
        .expect("a cut");
    assert!(made["marks"]["ceiling_estimate_tokens"].is_u64(), "{made}");
    assert!(made["marks"].get("ceiling_tokens").is_none(), "{made}");
}

/// #2414: the emergency fallback stays. In a loop built with no mode
/// switched on, a request no cut can bring under the ceiling (the pinned
/// brief alone is over it) runs the ladder, logged as the watermark
/// fallback, and is sent under the ceiling.
#[tokio::test]
async fn the_emergency_ladder_runs_in_a_loop_built_with_no_mode_set() {
    let (mut rig, sink) = audited(Rig::new(None));
    rig.agent = default_agent(rig.store.clone(), 20_000);
    rig.agent
        .set_audit_log(Some(sink.clone() as Arc<dyn AuditSink>));
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
    assert_eq!(prunes(&sink), [true], "{:?}", events(&sink));
    assert!(sent <= 20_000, "{sent} over the 20k ceiling");
}
