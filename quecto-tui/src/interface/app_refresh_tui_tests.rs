use super::tui_harness::TuiHarness;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

#[tokio::test]
async fn handle_submit_refresh_tui_updates_terminal_size_and_redraws_without_agent_command() {
    let mut h = harness().await;
    h.app_mut().terminal.set_size_for_tests(1, 1);
    let before = h.rendered_frames();
    h.app_mut().handle_submit("/refresh-tui");

    assert!(
        h.app_mut().terminal.width > 1 && h.app_mut().terminal.height > 1,
        "/refresh-tui should re-query terminal dimensions before redrawing"
    );
    assert_eq!(
        h.rendered_frames(),
        before + 1,
        "/refresh-tui should force one full redraw"
    );
    let cmds = h.drain_commands().await;
    assert!(
        cmds.is_empty(),
        "/refresh-tui should not send any agent commands: {cmds:?}"
    );
}
