use super::*;
use crate::domain::audit::AuditEvent;
use tempfile::TempDir;

#[tokio::test]
async fn creates_audit_directory_on_open() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path();
    assert!(!base.join("audit").exists());

    let _log = AuditLog::open(base, "test-session").await.unwrap();
    assert!(base.join("audit").exists());
}

#[tokio::test]
async fn writes_valid_jsonl_with_envelope() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open(tmp.path(), "cli:my-feature").await.unwrap();

    log.emit(
        7,
        AuditEvent::ToolCall {
            tool: "bash".into(),
            call_id: "call_abc".into(),
            arguments: r#"{"command":"test"}"#.into(),
            raw_arguments: None,
        },
    )
    .await
    .unwrap();

    let path = AuditLog::file_path(tmp.path(), "cli:my-feature");
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1);

    let val: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(val["session"], "cli:my-feature");
    assert_eq!(val["turn"], 7);
    assert_eq!(val["event"], "tool_call");
    assert_eq!(val["tool"], "bash");
    assert_eq!(val["call_id"], "call_abc");
    // ts should be ISO 8601
    let ts = val["ts"].as_str().unwrap();
    assert!(ts.ends_with('Z'));
    assert!(ts.contains('T'));
}

#[tokio::test]
async fn appends_multiple_events_in_order() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open(tmp.path(), "multi").await.unwrap();

    log.emit(
        1,
        AuditEvent::ToolCall {
            tool: "bash".into(),
            call_id: "c1".into(),
            arguments: "{}".into(),
            raw_arguments: None,
        },
    )
    .await
    .unwrap();
    log.emit(
        1,
        AuditEvent::ToolResult {
            call_id: "c1".into(),
            tool: "bash".into(),
            is_error: false,
            content_tokens: 100,
            content_preview: "ok".into(),
            duration_ms: 0,
            argument_bytes: 0,
            content_bytes: 0,
        },
    )
    .await
    .unwrap();
    log.emit(
        2,
        AuditEvent::LlmTurnStart {
            input_tokens_estimate: 5000,
            message_count: 10,
        },
    )
    .await
    .unwrap();
    log.emit(
        2,
        AuditEvent::LlmTurnEnd {
            usage_source: None,
            input_tokens: 5000,
            output_tokens: 500,
            stop_reason: "end_turn".into(),
            duration_ms: 2000,
            cached_input_tokens: None,
            cache_write_tokens: None,
        },
    )
    .await
    .unwrap();

    let path = AuditLog::file_path(tmp.path(), "multi");
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 4);

    let v0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let v1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    let v2: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    let v3: serde_json::Value = serde_json::from_str(lines[3]).unwrap();
    assert_eq!(v0["event"], "tool_call");
    assert_eq!(v1["event"], "tool_result");
    assert_eq!(v2["event"], "llm_turn_start");
    assert_eq!(v3["event"], "llm_turn_end");
}

#[tokio::test]
async fn flushes_on_every_write() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open(tmp.path(), "flush-test").await.unwrap();

    log.emit(
        1,
        AuditEvent::ToolCall {
            tool: "bash".into(),
            call_id: "c1".into(),
            arguments: "{}".into(),
            raw_arguments: None,
        },
    )
    .await
    .unwrap();

    // Read without closing the log — file should be readable
    let path = AuditLog::file_path(tmp.path(), "flush-test");
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(content.lines().count(), 1);

    // Emit another event
    log.emit(
        2,
        AuditEvent::ToolCall {
            tool: "read".into(),
            call_id: "c2".into(),
            arguments: "{}".into(),
            raw_arguments: None,
        },
    )
    .await
    .unwrap();

    let content2 = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(content2.lines().count(), 2);
}

#[tokio::test]
async fn sanitizes_session_key_for_filename() {
    let tmp = TempDir::new().unwrap();
    let _log = AuditLog::open(tmp.path(), "cli:my-feature").await.unwrap();

    let path = AuditLog::file_path(tmp.path(), "cli:my-feature");
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "cli_my-feature.jsonl"
    );
}

#[tokio::test]
async fn all_event_types_write_successfully() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open(tmp.path(), "all-types").await.unwrap();

    let events = vec![
        AuditEvent::ToolCall {
            tool: "bash".into(),
            call_id: "c1".into(),
            arguments: "{}".into(),
            raw_arguments: None,
        },
        AuditEvent::ToolResult {
            call_id: "c1".into(),
            tool: "bash".into(),
            is_error: false,
            content_tokens: 10,
            content_preview: "ok".into(),
            duration_ms: 0,
            argument_bytes: 0,
            content_bytes: 0,
        },
        AuditEvent::LlmTurnStart {
            input_tokens_estimate: 1000,
            message_count: 5,
        },
        AuditEvent::LlmTurnEnd {
            usage_source: None,
            input_tokens: 1000,
            output_tokens: 200,
            stop_reason: "end_turn".into(),
            duration_ms: 500,
            cached_input_tokens: None,
            cache_write_tokens: None,
        },
        AuditEvent::WorkflowStep {
            action: "check".into(),
            step_index: 1,
            step_key: "tests".into(),
            step_label: "Write tests".into(),
            template_id: "feature".into(),
        },
        AuditEvent::WorkflowTransition {
            from_mode: "selector".into(),
            to_mode: "active".into(),
            template_id: Some("feature".into()),
            issue: None,
        },
        AuditEvent::ContextPruned {
            messages_dropped: 5,
            tokens_before: 100_000,
            tokens_after: 80_000,
            budget_unmet: false,
            ladder_stubbed: 2,
            ceiling_tokens: 0,
            watermark_fallback: false,
        },
        AuditEvent::SubagentSpawned {
            agent_id: "reviewer".into(),
            task_preview: "Review code".into(),
            system_preview: "You are a reviewer".into(),
        },
        AuditEvent::SubagentCmd {
            agent_id: "reviewer".into(),
            command: "status".into(),
        },
        AuditEvent::GuardBlocked {
            command_preview: "git push".into(),
            guard_message: "Not ready".into(),
            before_step_key: "commit".into(),
        },
        AuditEvent::Error {
            source: "provider".into(),
            tool: None,
            message: "rate limited".into(),
            location: None,
        },
    ];

    for (i, event) in events.into_iter().enumerate() {
        log.emit(i as u32 + 1, event).await.unwrap();
    }

    let path = AuditLog::file_path(tmp.path(), "all-types");
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 11);

    // Verify each line is valid JSON with envelope
    for (i, line) in lines.iter().enumerate() {
        let val: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(val["ts"].is_string(), "line {} missing ts", i);
        assert_eq!(val["session"], "all-types");
        assert_eq!(val["turn"], i as u64 + 1);
        assert!(val["event"].is_string(), "line {} missing event", i);
    }
}

#[test]
fn open_sync_creates_sanitized_log_file_and_debug_redacts_path() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open_sync(tmp.path(), "cli:sync-session").unwrap();
    let path = AuditLog::file_path(tmp.path(), "cli:sync-session");

    assert!(path.exists(), "open_sync must create the append log file");
    assert_eq!(path.file_name().unwrap(), "cli_sync-session.jsonl");
    let debug = format!("{log:?}");
    assert!(
        debug.contains("cli:sync-session"),
        "debug should name the session: {debug}"
    );
    assert!(
        !debug.contains(tmp.path().to_str().unwrap()),
        "debug must not expose the filesystem path: {debug}"
    );
}

#[test]
fn unix_to_utc_handles_leap_year_boundaries_and_centuries() {
    assert_eq!(unix_to_utc(951_782_400), (2000, 2, 29, 0, 0, 0));
    assert_eq!(unix_to_utc(951_868_800), (2000, 3, 1, 0, 0, 0));
    assert!(is_leap(2000));
    assert!(!is_leap(1900));
    assert!(is_leap(2024));
    assert!(!is_leap(2025));
}

#[test]
fn unix_to_utc_epoch() {
    assert_eq!(unix_to_utc(0), (1970, 1, 1, 0, 0, 0));
}

#[test]
fn unix_to_utc_known_date() {
    // 2026-03-28T00:00:00Z = 1774656000
    let (y, m, d, h, mi, s) = unix_to_utc(1_774_656_000);
    assert_eq!((y, m, d, h, mi, s), (2026, 3, 28, 0, 0, 0));
}

#[test]
fn now_utc_iso8601_format() {
    let ts = now_utc_iso8601();
    assert!(ts.ends_with('Z'));
    assert!(ts.contains('T'));
    assert_eq!(ts.len(), 24); // "YYYY-MM-DDTHH:MM:SS.mmmZ"
}

/// #2150: the log is private: its directory 0700 and its files 0600, a
/// looser directory or file left by an earlier version tightened.
#[tokio::test]
async fn the_log_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let _log = AuditLog::open(tmp.path(), "s1").await.unwrap();
    assert_eq!(mode(&tmp.path().join("audit")), 0o700);
    assert_eq!(mode(&AuditLog::file_path(tmp.path(), "s1")), 0o600);
    let loose = AuditLog::file_path(tmp.path(), "s2");
    std::fs::write(&loose, "").unwrap();
    std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(
        tmp.path().join("audit"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let _log = AuditLog::open_sync(tmp.path(), "s2").unwrap();
    assert_eq!(mode(&tmp.path().join("audit")), 0o700);
    assert_eq!(mode(&loose), 0o600);
}

/// #2150: every record says when (Unix milliseconds), which process, and
/// for a sub-agent, its parent.
#[tokio::test]
async fn each_record_names_its_time_process_and_parent() {
    let tmp = TempDir::new().unwrap();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let log = AuditLog::open(tmp.path(), "child")
        .await
        .unwrap()
        .with_parent(Some("chat-parent".into()));
    log.emit(
        1,
        AuditEvent::SubagentCmd {
            agent_id: "a".into(),
            command: "c".into(),
        },
    )
    .await
    .unwrap();
    let text = std::fs::read_to_string(AuditLog::file_path(tmp.path(), "child")).unwrap();
    let record: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert!(record["unix_ms"].as_u64().unwrap() >= before, "{record}");
    assert_eq!(record["pid"], std::process::id());
    assert_eq!(record["parent"], "chat-parent");
    // #2161: the host tells containers apart, whose pids repeat.
    let host = std::process::Command::new("hostname").output().unwrap();
    assert_eq!(record["host"], String::from_utf8_lossy(&host.stdout).trim());
}

/// #2150: a log stops at its size cap with one final record saying so;
/// nothing is written after it.
#[tokio::test]
async fn a_log_stops_at_its_cap_with_one_final_record() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open(tmp.path(), "big")
        .await
        .unwrap()
        .with_cap(600);
    for n in 0..20 {
        log.emit(
            n,
            AuditEvent::SubagentCmd {
                agent_id: "a".into(),
                command: "x".repeat(40),
            },
        )
        .await
        .unwrap();
    }
    let text = std::fs::read_to_string(AuditLog::file_path(tmp.path(), "big")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let last: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    assert_eq!(last["event"], "log_capped", "{text}");
    assert_eq!(last["cap_bytes"], 600);
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("log_capped"))
            .count(),
        1
    );
    let before_cap: usize = lines[..lines.len() - 1]
        .iter()
        .map(|line| line.len() + 1)
        .sum();
    assert!(before_cap <= 600, "{before_cap}");
}

/// #2150: `~/.quecto` is shared with containers, so a link planted as the
/// audit directory or a log file is never followed: the open fails and
/// what the link names is untouched.
#[tokio::test]
async fn a_planted_link_is_never_followed() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let elsewhere = TempDir::new().unwrap();
    let victim = elsewhere.path().join("tool");
    std::fs::write(&victim, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::create_dir(tmp.path().join("audit")).unwrap();
    std::os::unix::fs::symlink(&victim, AuditLog::file_path(tmp.path(), "s")).unwrap();
    assert!(AuditLog::open_sync(tmp.path(), "s").is_err());
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "#!/bin/sh\n");
    assert_eq!(
        std::fs::metadata(&victim).unwrap().permissions().mode() & 0o777,
        0o755
    );
    std::fs::set_permissions(elsewhere.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let linked = TempDir::new().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), linked.path().join("audit")).unwrap();
    assert!(AuditLog::open_sync(linked.path(), "s").is_err());
    assert_eq!(
        std::fs::metadata(elsewhere.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755,
        "a linked directory is never made private in its target's place"
    );
    assert!(!elsewhere.path().join("s.jsonl").exists());
}

/// #2150: a log that reached its cap stays stopped when its session
/// restarts: no second `log_capped` record.
#[tokio::test]
async fn a_capped_log_stays_stopped_after_a_restart() {
    let tmp = TempDir::new().unwrap();
    let emit_all = |log: AuditLog| async move {
        for n in 0..20 {
            log.emit(
                n,
                AuditEvent::SubagentCmd {
                    agent_id: "a".into(),
                    command: "x".repeat(40),
                },
            )
            .await
            .unwrap();
        }
    };
    emit_all(
        AuditLog::open(tmp.path(), "big")
            .await
            .unwrap()
            .with_cap(600),
    )
    .await;
    emit_all(
        AuditLog::open(tmp.path(), "big")
            .await
            .unwrap()
            .with_cap(600),
    )
    .await;
    let text = std::fs::read_to_string(AuditLog::file_path(tmp.path(), "big")).unwrap();
    assert_eq!(text.matches("log_capped").count(), 1, "{text}");
}

/// #2150 review: a FIFO planted as a log file is refused at once, never
/// waited on: only a regular file is a log.
#[tokio::test]
async fn a_planted_fifo_is_refused_without_waiting() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("audit")).unwrap();
    let fifo =
        std::ffi::CString::new(AuditLog::file_path(tmp.path(), "s").to_str().unwrap()).unwrap();
    // SAFETY: a NUL-terminated path; mkfifo creates the node only.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let started = std::time::Instant::now();
    assert!(AuditLog::open_sync(tmp.path(), "s").is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

/// #2150 review: the `log_capped` record replaces a line that did not
/// fit, often far shorter, leaving the file below its cap: it still stays
/// stopped after a restart.
#[tokio::test]
async fn a_log_capped_by_a_large_line_stays_stopped_after_a_restart() {
    let tmp = TempDir::new().unwrap();
    let small = || AuditEvent::SubagentCmd {
        agent_id: "a".into(),
        command: "x".into(),
    };
    let log = AuditLog::open(tmp.path(), "big")
        .await
        .unwrap()
        .with_cap(2_000);
    log.emit(1, small()).await.unwrap();
    log.emit(
        2,
        AuditEvent::SubagentCmd {
            agent_id: "a".into(),
            command: "y".repeat(5_000),
        },
    )
    .await
    .unwrap();
    drop(log);
    let log = AuditLog::open(tmp.path(), "big")
        .await
        .unwrap()
        .with_cap(2_000);
    log.emit(3, small()).await.unwrap();
    let text = std::fs::read_to_string(AuditLog::file_path(tmp.path(), "big")).unwrap();
    assert!(
        text.len() < 2_000,
        "the cap record left it short: {}",
        text.len()
    );
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(
        text.lines().last().unwrap().contains("log_capped"),
        "{text}"
    );
}

/// #2184 review: a failed call, an unterminated or an empty name gives no
/// host.
#[test]
fn a_host_name_only_from_a_good_call() {
    assert_eq!(super::host_from(0, b"box\0junk"), Some("box".into()));
    assert_eq!(super::host_from(-1, b"box\0"), None);
    assert_eq!(super::host_from(0, b"unterminated"), None);
    assert_eq!(super::host_from(0, b"\0"), None);
}

#[test]
fn the_crash_line_stays_inside_the_logs_cap() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:capped")
        .unwrap()
        .with_cap(64);
    let error = log
        .crash_line()
        .unwrap()
        .write(
            0,
            AuditEvent::Error {
                source: "panic".into(),
                tool: None,
                message: "does not fit".into(),
                location: None,
            },
        )
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::StorageFull);
    let path = AuditLog::file_path(base.path(), "cli:capped");
    assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
}

/// #2192 review (F1): a log the async writer capped — with room still to
/// spare — takes no crash line after its `log_capped` record.
#[tokio::test]
async fn the_crash_line_writes_nothing_after_the_log_was_capped() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:capped")
        .unwrap()
        .with_cap(1024);
    let crash_line = log.crash_line().unwrap();
    let error = |message: String| AuditEvent::Error {
        source: "panic".into(),
        tool: None,
        message,
        location: None,
    };
    log.emit(0, error("x".repeat(2000))).await.unwrap();
    let path = AuditLog::file_path(base.path(), "cli:capped");
    let held = std::fs::metadata(&path).unwrap().len();
    assert!(held < 512, "capped with room to spare: {held}");
    let refused = crash_line.write(1, error("small".into())).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::StorageFull);
    let text = std::fs::read_to_string(&path).unwrap();
    let last = text.lines().last().unwrap();
    assert!(last.contains(r#""event":"log_capped""#), "{text}");
    // A log reopened already capped refuses it too.
    let reopened = AuditLog::open_sync(base.path(), "cli:capped").unwrap();
    assert!(
        reopened
            .crash_line()
            .unwrap()
            .write(1, error("small".into()))
            .is_err()
    );
}

fn panic_event(message: &str) -> AuditEvent {
    AuditEvent::Error {
        source: "panic".into(),
        tool: None,
        message: message.into(),
        location: None,
    }
}

/// #2192 review: the crash line's bytes count against the cap the async
/// writer keeps — a line it wrote is not room the async writer still has.
#[tokio::test]
async fn a_crash_line_written_counts_against_the_async_writers_cap() {
    let base = tempfile::tempdir().unwrap();
    let line_len = |message: &str| {
        envelope_line("cli:budget", None, Some(0), panic_event(message))
            .unwrap()
            .len() as u64
    };
    // Room for two lines of this size, and no more.
    let cap = 2 * line_len("x".repeat(40).as_str()) + 8;
    let log = AuditLog::open_sync(base.path(), "cli:budget")
        .unwrap()
        .with_cap(cap);
    log.crash_line()
        .unwrap()
        .write(0, panic_event(&"x".repeat(40)))
        .unwrap();
    log.emit(0, panic_event(&"y".repeat(40))).await.unwrap();
    // The async writer alone would think one more fits; with the crash
    // line's bytes counted it does not, and the log caps.
    log.emit(0, panic_event("z")).await.unwrap();
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:budget")).unwrap();
    let last = text.lines().last().unwrap();
    assert!(last.contains(r#""event":"log_capped""#), "{text}");
    assert!(!text.contains(r#""message":"z""#), "{text}");
}

/// #2192 review: bytes the async writer has reserved — its line not yet
/// on disk — are not room the crash line may take: the cap is exact.
#[test]
fn a_line_reserved_but_not_yet_written_leaves_the_crash_line_no_room() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:reserved")
        .unwrap()
        .with_cap(4096);
    let crash_line = log.crash_line().unwrap();
    // As the async writer does, just before its write lands.
    assert!(reserve(&log.gate.reserved, 4000, 4096));
    let path = AuditLog::file_path(base.path(), "cli:reserved");
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        0,
        "nothing on disk"
    );
    let refused = crash_line
        .write(0, panic_event("does not fit"))
        .unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::StorageFull);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
}

#[test]
fn a_reservation_is_taken_whole_or_not_at_all() {
    let budget = std::sync::atomic::AtomicU64::new(10);
    assert!(reserve(&budget, 5, 20));
    assert!(!reserve(&budget, 6, 20), "15 + 6 passes 20");
    assert!(reserve(&budget, 5, 20), "exactly the cap fits");
    assert!(!reserve(&budget, 1, 20));
    assert!(!reserve(
        &std::sync::atomic::AtomicU64::new(u64::MAX),
        1,
        u64::MAX
    ));
    assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire), 20);
}

/// #2404: a watermark cut's records land in the event log as one line
/// each, under the envelope, and read back whole.
#[tokio::test]
async fn context_cut_records_are_written_to_the_event_log() {
    use crate::domain::audit::{
        ContextCutRecord, ContextCutSkippedRecord, CutMarks, CutSkipReason,
    };
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open(tmp.path(), "watermark").await.unwrap();
    let marks = CutMarks {
        high_tokens: 256_000,
        low_tokens: 70_000,
        ceiling_estimate_tokens: 300_000,
        ceiling_lowered_marks: false,
        estimate_scale_permille: 1_000,
    };
    let cut = AuditEvent::ContextCut(ContextCutRecord {
        tokens_before: 257_000,
        tokens_after: 69_000,
        marks,
        messages_archived: 300,
        messages_kept: 20,
        archive_id: Some("archive".into()),
        fill: crate::domain::conversation::watermark::Fill::WithinLow,
    });
    let skipped = AuditEvent::ContextCutSkipped(ContextCutSkippedRecord {
        tokens: 258_000,
        marks,
        reason: CutSkipReason::NoBoundary,
    });
    log.emit(4, cut.clone()).await.unwrap();
    log.emit(5, skipped.clone()).await.unwrap();
    let path = AuditLog::file_path(tmp.path(), "watermark");
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2, "{content}");
    let envelopes: Vec<crate::domain::audit::AuditEnvelope> = lines
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(envelopes[0].event, cut);
    assert_eq!(envelopes[0].turn, Some(4));
    assert_eq!(envelopes[1].event, skipped);
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["event"], "context_cut");
    assert_eq!(first["archive_id"], "archive");
    assert_eq!(first["fill"], "within_low");
}
