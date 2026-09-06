use super::tui_harness::*;
use crate::components::ansi::strip_ansi;
use crate::protocol::client::Event;

fn top_region(h: &mut TuiHarness) -> String {
    h.main_pane()
}

#[tokio::test]
async fn unnamed_active_tab_labels_main_pane_as_coordinator() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);

    let top = strip_ansi(&top_region(&mut h));
    let title_line = top
        .lines()
        .find(|line| line.contains(" · ") && (line.contains("idle") || line.contains("running")))
        .unwrap_or_else(|| panic!("coordinator main-pane title not found:\n{top}"));
    assert!(
        title_line.contains("Coordinator"),
        "unnamed master/coordinator selection must render as Coordinator: {title_line:?}"
    );
    assert!(
        !title_line.contains("Master"),
        "unnamed coordinator main-pane title must not render legacy Master label: {title_line:?}"
    );
}

#[tokio::test]
async fn named_active_tab_labels_master_surfaces() {
    let mut h = TuiHarness::new().await;
    h.app_mut().ac_mut().name = Some("Investigate auth".into());
    h.event(Event::AgentStart);

    let panel = strip_ansi(
        &h.app_mut()
            .render_subagent_panel(30, 24, tokio::time::Instant::now())
            .join("\n"),
    );
    let coordinator_row = panel
        .lines()
        .find(|line| line.contains("Investigate auth") || line.contains("Coordinator"))
        .unwrap_or_else(|| panic!("coordinator row not found in panel:\n{panel}"));
    assert!(
        coordinator_row.contains("Investigate auth"),
        "a named active tab must label the pinned coordinator row with the tab name: {coordinator_row:?}"
    );
    assert!(
        !coordinator_row.contains("Coordinator"),
        "the fallback coordinator-row label is only for unnamed N=1 tabs: {coordinator_row:?}"
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
