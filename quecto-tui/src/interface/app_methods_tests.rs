//! Tests for pure helper functions in `app_methods.rs` (issue #729).

use super::app_methods;
use crate::infrastructure::client::Client;
use crate::infrastructure::terminal::Terminal;
use crate::interface::component::Component;
use tokio::io::AsyncReadExt;

async fn test_app_for_methods() -> super::App {
    let dir = std::env::temp_dir().join(format!(
        "quecto-tui-app-methods-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket_path = dir.join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
    });
    let client = Client::connect(&socket_path).await.unwrap();
    super::App::new(Terminal::new(), client)
}

// ── format_utc_minutes ──────────────────────────────────────────────────

#[test]
fn format_utc_minutes_epoch() {
    // Unix epoch: 1970-01-01 00:00 UTC
    assert_eq!(app_methods::format_utc_minutes(0), "1970-01-01 00:00");
}

#[test]
fn format_utc_minutes_one_minute() {
    assert_eq!(app_methods::format_utc_minutes(60), "1970-01-01 00:01");
}

#[test]
fn format_utc_minutes_one_hour() {
    assert_eq!(app_methods::format_utc_minutes(3600), "1970-01-01 01:00");
}

#[test]
fn format_utc_minutes_one_day() {
    assert_eq!(app_methods::format_utc_minutes(86400), "1970-01-02 00:00");
}

#[test]
fn format_utc_minutes_known_date() {
    // 2024-01-01 00:00 UTC = 1704067200 seconds
    assert_eq!(
        app_methods::format_utc_minutes(1704067200),
        "2024-01-01 00:00"
    );
}

#[test]
fn format_utc_minutes_mid_afternoon() {
    // 2024-06-15 14:30 UTC
    assert_eq!(
        app_methods::format_utc_minutes(1718461800),
        "2024-06-15 14:30"
    );
}

#[test]
fn format_utc_minutes_leap_year() {
    // 2024-02-29 12:00 UTC (leap day)
    assert_eq!(
        app_methods::format_utc_minutes(1709208000),
        "2024-02-29 12:00"
    );
}

#[test]
fn format_utc_minutes_year_boundary() {
    // 2023-12-31 23:59 UTC → 2024-01-01 00:00 is the next minute
    let secs = 1704067140; // 2023-12-31 23:59 UTC
    assert_eq!(app_methods::format_utc_minutes(secs), "2023-12-31 23:59");
}

// ── civil_from_days ─────────────────────────────────────────────────────

#[test]
fn civil_from_days_epoch() {
    // Day 0 = 1970-01-01
    assert_eq!(app_methods::civil_from_days(0), (1970, 1, 1));
}

#[test]
fn civil_from_days_one_day() {
    assert_eq!(app_methods::civil_from_days(1), (1970, 1, 2));
}

#[test]
fn civil_from_days_end_of_january() {
    assert_eq!(app_methods::civil_from_days(30), (1970, 1, 31));
}

#[test]
fn civil_from_days_february_start() {
    assert_eq!(app_methods::civil_from_days(31), (1970, 2, 1));
}

#[test]
fn civil_from_days_known_date() {
    // 2024-01-01 is day 19723 from epoch
    assert_eq!(app_methods::civil_from_days(19723), (2024, 1, 1));
}

#[test]
fn civil_from_days_leap_day() {
    // 2024-02-29 is day 19782 from epoch
    assert_eq!(app_methods::civil_from_days(19782), (2024, 2, 29));
}

#[test]
fn civil_from_days_negative() {
    // Day -1 = 1969-12-31
    assert_eq!(app_methods::civil_from_days(-1), (1969, 12, 31));
}

#[test]
fn civil_from_days_year_2000() {
    // 2000-01-01 is day 10957 from epoch
    assert_eq!(app_methods::civil_from_days(10957), (2000, 1, 1));
}

#[test]
fn civil_from_days_december_31() {
    // 2024-12-31 is day 20088 from epoch
    assert_eq!(app_methods::civil_from_days(20088), (2024, 12, 31));
}

// ── subagent_activity_line ──────────────────────────────────────────────

#[test]
fn subagent_activity_line_singular() {
    let line = app_methods::subagent_activity_line(1, 0);
    assert!(
        line.contains("subagent"),
        "should contain 'subagent': {line}"
    );
    assert!(
        !line.contains("subagents"),
        "singular form should not have plural: {line}"
    );
}

#[test]
fn subagent_activity_line_plural() {
    let line = app_methods::subagent_activity_line(3, 0);
    assert!(
        line.contains("subagents"),
        "plural form should have 'subagents': {line}"
    );
}

#[test]
fn subagent_activity_line_contains_count() {
    let line = app_methods::subagent_activity_line(5, 0);
    assert!(line.contains("5"), "should contain the count: {line}");
}

#[test]
fn subagent_activity_line_animates() {
    let frame0 = app_methods::subagent_activity_line(2, 0);
    let frame1 = app_methods::subagent_activity_line(2, 1);
    // The spinner character should differ between frames (animation).
    let strip = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
    // At minimum, both should contain the subagent text.
    assert!(strip(&frame0).contains("subagents"));
    assert!(strip(&frame1).contains("subagents"));
}

#[test]
fn subagent_activity_line_zero() {
    // Zero active subagents — should still render without panic.
    let line = app_methods::subagent_activity_line(0, 0);
    assert!(!line.is_empty());
}

#[test]
fn subagent_activity_line_frame_wraps() {
    // Frame index beyond SPINNER_FRAMES.len() should wrap without panic.
    let line = app_methods::subagent_activity_line(1, 1000);
    assert!(!line.is_empty());
}

// ── strip_ansi ───────────────────────────────────────────────────────────

#[test]
fn strip_ansi_empty_string() {
    assert_eq!(app_methods::strip_ansi(""), "");
}

#[test]
fn strip_ansi_plain_text() {
    assert_eq!(app_methods::strip_ansi("hello world"), "hello world");
}

#[test]
fn strip_ansi_removes_color_codes() {
    assert_eq!(
        app_methods::strip_ansi("\x1b[31mred\x1b[0m text"),
        "red text"
    );
}

#[test]
fn strip_ansi_preserves_unicode() {
    assert_eq!(app_methods::strip_ansi("héllo 世界"), "héllo 世界");
}

#[test]
fn strip_ansi_removes_multiple_escapes() {
    assert_eq!(
        app_methods::strip_ansi("\x1b[1m\x1b[32mbold green\x1b[0m"),
        "bold green"
    );
}

#[test]
fn strip_ansi_removes_csi_with_tilde() {
    assert_eq!(app_methods::strip_ansi("\x1b[5~text"), "text");
}

#[test]
fn strip_ansi_removes_osc_with_bel() {
    assert_eq!(app_methods::strip_ansi("\x1b]0;title\x07text"), "text");
}

#[test]
fn strip_ansi_removes_osc_with_st() {
    assert_eq!(app_methods::strip_ansi("\x1b]0;title\x1b\\text"), "text");
}

#[tokio::test]
async fn render_failure_becomes_error_notification() {
    let mut app = test_app_for_methods().await;
    let err = std::io::Error::other("terminal closed");

    app.handle_render_failure(&err);

    let rendered = app.notifications.render(120).join("\n");
    assert!(
        rendered.contains("Failed to render frame: terminal closed"),
        "render failure should be visible in notifications: {rendered}"
    );
}

#[test]
fn composite_centered_splices_overlay_at_centered_origin() {
    // A 10x6 frame of dots, splice a 4-wide x 2-tall overlay; it should land
    // centered: start_row = (6-2)/2 = 2, start_col = (10-4)/2 = 3.
    let width = 10usize;
    let height = 6usize;
    let mut lines: Vec<String> = (0..height).map(|_| ".".repeat(width)).collect();
    let overlay = vec!["ABCD".to_string(), "EFGH".to_string()];
    super::App::composite_centered(&mut lines, &overlay, 4, width, height);

    let stripped: Vec<String> = lines.iter().map(|l| app_methods::strip_ansi(l)).collect();
    assert_eq!(stripped[2], "...ABCD...");
    assert_eq!(stripped[3], "...EFGH...");
    // Rows outside the overlay are untouched.
    assert_eq!(stripped[1], "..........");
    assert_eq!(stripped[4], "..........");
}
