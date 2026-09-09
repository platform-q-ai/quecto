use super::*;
use crate::components::{ansi::strip_ansi, utils::visible_width};
use crate::shell::app::tui_harness::TuiHarness;
use crate::shell::app::tui_harness::{subagent, subagents_changed};

#[tokio::test(start_paused = true)]
async fn admission_layout_reserves_compact_timer_before_names() {
    let mut harness = TuiHarness::new().await;
    let app = harness.app_mut();
    let mut child = subagent("child", "running", None);
    child.read_only = true;
    app.handle_event(subagents_changed(vec![child]));
    for name in [
        "a",
        "abcdefghijkl",
        "a-very-long-child-name-that-must-truncate",
        "界界界界界界界界",
        "e\u{301}e\u{301}e\u{301}長い名前",
    ] {
        for prefix in ["├ ", "│ └ ", "│ │ │ └ "] {
            for seconds in [9, 10, 99, 100] {
                let row = PanelRow {
                    id: Some("child".into()),
                    env_key: None,
                    prefix: prefix.into(),
                    label: name.into(),
                    status: "running".into(),
                    admission: Some(format!("{seconds}s")),
                    workflow: None,
                };
                let line = strip_ansi(&app.panel_name_line(
                    &row,
                    true,
                    false,
                    34,
                    tokio::time::Instant::now(),
                ));
                assert!(
                    line.contains(&format!("⏳ {seconds}s")),
                    "{name}/{prefix}: {line}"
                );
                assert!(line.contains("⊘"), "{line}");
                assert!(line.contains("0:00"), "{line}");
                assert_eq!(visible_width(&line), 34);
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn admission_layout_narrow_fallback_preserves_status_and_exact_width() {
    let mut harness = TuiHarness::new().await;
    let app = harness.app_mut();
    let row = PanelRow {
        id: None,
        env_key: None,
        prefix: "│ │ │ └ ".into(),
        label: "界long-child".into(),
        status: "running".into(),
        admission: Some("100s".into()),
        workflow: None,
    };
    for width in 0..35 {
        let line =
            strip_ansi(&app.panel_name_line(&row, true, false, width, tokio::time::Instant::now()));
        assert_eq!(visible_width(&line), width, "{line:?}");
        match width {
            0 => assert_eq!(line, ""),
            1 => assert_eq!(line, "?"),
            2..=6 => assert!(line.contains("⏳"), "{width}: {line}"),
            _ => assert!(line.contains("⏳ 100s"), "{width}: {line}"),
        }
    }
}

#[tokio::test(start_paused = true)]
async fn admission_layout_ordinary_elapsed_timer_is_unchanged() {
    let mut harness = TuiHarness::new().await;
    let app = harness.app_mut();
    let row = PanelRow {
        id: None,
        env_key: None,
        prefix: "".into(),
        label: "Coordinator".into(),
        status: "idle".into(),
        admission: None,
        workflow: None,
    };
    let line =
        strip_ansi(&app.panel_name_line(&row, false, false, 34, tokio::time::Instant::now()));
    assert!(line.starts_with(" Coordinator"), "{line}");
    assert!(line.ends_with("0:00 "), "{line}");
    assert_eq!(visible_width(&line), 34);
}

#[tokio::test(start_paused = true)]
async fn admission_layout_headless_panel_keeps_elapsed_wait_across_digit_transitions() {
    use crate::protocol::client::Event;
    let mut harness = TuiHarness::new().await;
    let name = "abcdefghijkl-long-observer";
    let mut child = subagent(name, "running", None);
    child.read_only = true;
    harness
        .app_mut()
        .handle_event(subagents_changed(vec![child]));
    for start in [9, 99] {
        harness.app_mut().handle_event(Event::AdmissionStateChanged {
            agent_id: Some(name.into()),
            admission: serde_json::json!({"waiting": 1, "longestWaitSeconds": start, "revision": start}),
        });
        for elapsed in 0..=1 {
            if elapsed == 1 {
                tokio::time::advance(std::time::Duration::from_secs(1)).await;
                harness.app_mut().tick_admission_labels();
            }
            let panel = harness.left_panel();
            assert!(
                panel.contains(&format!("⏳ {}s", start + elapsed)),
                "{panel}"
            );
            assert!(panel.contains('…'), "long name must truncate: {panel}");
            assert!(panel.contains('⊘'), "observer must remain: {panel}");
        }
    }
}
