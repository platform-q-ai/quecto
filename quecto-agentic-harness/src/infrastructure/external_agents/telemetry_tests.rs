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
