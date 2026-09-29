//! The event-log adapter (#2304): it files each record the log keeps, in
//! order, and a write that fails never fails the member.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use super::*;
use crate::domain::error::DomainError;

#[derive(Default)]
struct Sink {
    written: Mutex<Vec<(u32, AuditEvent)>>,
    fail: bool,
}

impl AuditSink for Sink {
    fn emit(
        &self,
        turn: u32,
        event: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async move {
            match self.fail {
                true => Err(DomainError::Session("disk full".into())),
                false => {
                    self.written.lock().unwrap().push((turn, event));
                    Ok(())
                }
            }
        })
    }
}

fn member() -> MemberIdentity {
    MemberIdentity {
        member_ref: "C2".into(),
        credential_mode: "oauth_token",
    }
}

fn interrupted(turn: u64) -> SessionRecord {
    SessionRecord::Interrupted {
        turn,
        cause: "abort",
    }
}

#[test]
fn each_record_the_log_keeps_is_filed_in_order_once_the_adapter_is_dropped() {
    let sink = Arc::new(Sink::default());
    let telemetry = EventLogExternalAgentTelemetry::new(sink.clone(), member()).unwrap();
    telemetry.record(&SessionRecord::Started);
    telemetry.record(&SessionRecord::FollowUpStarted { turn: 1, bytes: 3 });
    for turn in 1..=50 {
        telemetry.record(&interrupted(turn));
    }
    drop(telemetry);
    let written = sink.written.lock().unwrap();
    let turns: Vec<u32> = written.iter().map(|(turn, _)| *turn).collect();
    assert_eq!(turns, [0].into_iter().chain(1..=50).collect::<Vec<u32>>());
    assert!(matches!(
        &written[0].1,
        AuditEvent::ExternalAgentLifecycle { member_ref, .. } if member_ref == "C2"
    ));
}

#[tokio::test]
async fn a_failing_log_never_fails_the_member_and_is_counted() {
    let sink = Arc::new(Sink {
        fail: true,
        ..Sink::default()
    });
    let telemetry = EventLogExternalAgentTelemetry::new(sink, member()).unwrap();
    for turn in 1..=3 {
        telemetry.record(&interrupted(turn));
    }
    let failures = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while telemetry.failures() < 3 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        telemetry.failures()
    })
    .await
    .expect("every failure is counted");
    assert_eq!(failures, 3);
    assert!(
        !telemetry.health.warning_due.load(Ordering::Relaxed),
        "warned, and not again"
    );
}

/// A sink whose first write waits for `release`, then fails; every later
/// write succeeds.
struct GatedSink {
    written: Mutex<Vec<(u32, AuditEvent)>>,
    entered: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    first: std::sync::atomic::AtomicBool,
}

impl AuditSink for GatedSink {
    fn emit(
        &self,
        turn: u32,
        event: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async move {
            if self.first.swap(false, Ordering::SeqCst) {
                if let Some(entered) = self.entered.lock().unwrap().take() {
                    entered.send(()).unwrap();
                }
                self.release.lock().unwrap().recv().unwrap();
                return Err(DomainError::Session("disk full".into()));
            }
            self.written.lock().unwrap().push((turn, event));
            Ok(())
        })
    }
}

#[test]
fn a_log_that_lost_records_ends_with_one_counting_what_it_dropped_and_failed() {
    let (entered, writer_entered) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let sink = Arc::new(GatedSink {
        written: Mutex::default(),
        entered: Mutex::new(Some(entered)),
        release: Mutex::new(released),
        first: std::sync::atomic::AtomicBool::new(true),
    });
    let telemetry = EventLogExternalAgentTelemetry::new(sink.clone(), member()).unwrap();
    // The writer holds the first record: the queue fills behind it.
    telemetry.record(&interrupted(1));
    writer_entered.recv().unwrap();
    for turn in 2..(2 + EVENT_LOG_QUEUE_CAPACITY as u64 + 3) {
        telemetry.record(&interrupted(turn));
    }
    release.send(()).unwrap();
    drop(telemetry);
    let written = sink.written.lock().unwrap();
    assert_eq!(written.len(), EVENT_LOG_QUEUE_CAPACITY + 1);
    let (turn, last) = written.last().unwrap();
    assert_eq!(*turn, 0);
    assert_eq!(
        last,
        &AuditEvent::ExternalAgentLifecycle {
            member_ref: "C2".into(),
            record: ExternalAgentLifecycle::LogIncomplete {
                dropped: 3,
                failed: 1,
            },
        }
    );
}

#[test]
fn a_log_that_lost_nothing_writes_no_such_record() {
    let sink = Arc::new(Sink::default());
    let telemetry = EventLogExternalAgentTelemetry::new(sink.clone(), member()).unwrap();
    telemetry.record(&interrupted(1));
    drop(telemetry);
    assert_eq!(sink.written.lock().unwrap().len(), 1);
}

/// A sink whose write never ends, or panics.
struct StuckSink {
    panics: bool,
}

impl AuditSink for StuckSink {
    fn emit(
        &self,
        _turn: u32,
        _event: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let panics = self.panics;
        Box::pin(async move {
            assert!(!panics, "the sink panics");
            std::future::pending().await
        })
    }
}

/// `finish` on another thread, answered within `bound` or `None`.
fn finish_within(
    mut telemetry: EventLogExternalAgentTelemetry,
    bound: std::time::Duration,
) -> Option<WriterEnd> {
    let (sent, answer) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let end = telemetry.finish();
        let _ = sent.send(end);
    });
    answer.recv_timeout(bound).ok()
}

#[test]
fn a_wedged_writer_holds_the_adapter_s_end_no_longer_than_its_bound() {
    let telemetry =
        EventLogExternalAgentTelemetry::new(Arc::new(StuckSink { panics: false }), member())
            .unwrap()
            .with_drain_bound(std::time::Duration::from_millis(100));
    telemetry.record(&interrupted(1));
    assert_eq!(
        finish_within(telemetry, std::time::Duration::from_secs(5)),
        Some(WriterEnd::TimedOut)
    );
}

#[test]
fn a_writer_that_panicked_is_told_apart_in_any_build() {
    let telemetry =
        EventLogExternalAgentTelemetry::new(Arc::new(StuckSink { panics: true }), member())
            .unwrap();
    telemetry.record(&interrupted(1));
    assert_eq!(
        finish_within(telemetry, std::time::Duration::from_secs(5)),
        Some(WriterEnd::Panicked)
    );
}
