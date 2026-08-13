use super::tui_harness::*;
use crate::components::ansi::strip_ansi;
use crate::protocol::client::Event;

fn top_region(h: &mut TuiHarness) -> String {
    h.main_pane()
}

#[tokio::test]
async fn named_active_tab_labels_master_surfaces() {
    let mut h = TuiHarness::new().await;
    h.app_mut().conn.name = Some("Investigate auth".into());
    h.event(Event::AgentStart);

    let panel = strip_ansi(
        &h.app_mut()
            .render_subagent_panel(30, 24, tokio::time::Instant::now())
            .join("\n"),
    );
    let master_row = panel
        .lines()
        .find(|line| line.contains("Investigate auth") || line.contains("Master Agent"))
        .unwrap_or_else(|| panic!("master row not found in panel:\n{panel}"));
    assert!(
        master_row.contains("Investigate auth"),
        "a named active tab must label the pinned master row with the tab name: {master_row:?}"
    );
    assert!(
        !master_row.contains("Master Agent"),
        "the legacy master-row label is only for unnamed N=1 tabs: {master_row:?}"
    );

    let top = strip_ansi(&top_region(&mut h));
    let title_line = top
        .lines()
        .find(|line| line.contains(" · ") && (line.contains("idle") || line.contains("running")))
        .unwrap_or_else(|| panic!("master main-pane title not found:\n{top}"));
    assert!(
        title_line.contains("Investigate auth"),
        "a named active tab must label the master main-pane title with the tab name: {title_line:?}"
    );
}
