use super::tui_harness::*;
use crate::components::ansi::strip_ansi;
use crate::protocol::client::Event;

fn top_region(h: &mut TuiHarness) -> String {
    h.main_pane()
}

#[tokio::test]
async fn main_pane_title_labels_the_master_as_coordinator() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);

    let top = strip_ansi(&top_region(&mut h));
    let title_line = top
        .lines()
        .find(|line| line.contains(" · ") && (line.contains("idle") || line.contains("running")))
        .unwrap_or_else(|| panic!("coordinator main-pane title not found:\n{top}"));
    assert!(
        title_line.contains("Coordinator"),
        "the master selection must render as Coordinator: {title_line:?}"
    );
    assert!(
        !title_line.contains("Master"),
        "the coordinator main-pane title must not render legacy Master label: {title_line:?}"
    );
}
