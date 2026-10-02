//! #2404: the watermark context's event-log records. A cut writes one
//! `context_cut` record (never a `context_pruned` one); a cut that is due
//! but not made writes a `context_cut_skipped` record naming why; the
//! emergency ladder writes a `context_pruned` record marked
//! `watermark_fallback`. Every record holds counts, ids and kinds only.

use super::ctx_mgmt_tests::CapturingAuditSink;
use super::watermark_tests::{HIGH, LOW, Rig, agent_under, stubs, text, total};
use crate::application::audit::ports::AuditSink;
use crate::domain::audit::{AuditEvent, ContextCutRecord, ContextCutSkippedRecord, CutSkipReason};
use crate::domain::conversation::ContextMode;
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
    let marks = ContextMode::Watermark(Watermark::new(100_000, 30_000).unwrap());
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
    assert_eq!(marks.ceiling_tokens, ceiling);
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
    assert_eq!(record.reason, CutSkipReason::SavingTooSmall);
    assert_eq!(record.needed_tokens, Some(HIGH / 10));
    let saving = record.saving_tokens.expect("the saving is named");
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
/// `context_pruned` record marked `watermark_fallback`; the default mode's
/// ladder writes one unmarked, and the default mode writes no cut records.
#[tokio::test]
async fn the_ladder_fallback_is_a_context_pruned_record_marked_as_the_fallback() {
    let marks = ContextMode::Watermark(Watermark::new(100_000, 30_000).unwrap());
    for (mode, fallback) in [(Some(marks), true), (None, false)] {
        let (mut rig, sink) = audited(Rig::new(None));
        rig.agent = agent_under(rig.store.clone(), mode, 20_000, None);
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
        assert_eq!(prunes(&sink), [fallback], "{mode:?}");
        match mode {
            Some(_) => {}
            None => assert!(cuts(&sink).is_empty() && skips(&sink).is_empty()),
        }
    }
}
