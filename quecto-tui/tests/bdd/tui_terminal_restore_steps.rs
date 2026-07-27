//! Steps for `tui_terminal_restore.feature`.
//!
//! Only the observable-state scenario ("modifyOtherKeys disabled on exit") is
//! wired: it drives the REAL on-exit protocol teardown (`KittyProtocol::cleanup`
//! via the headless harness) and asserts the real `modify_other_keys` flag is
//! reset to mode 0. The other scenarios assert raw terminal escape output /
//! termios cooked-mode restoration, which the production teardown writes
//! straight to the process stdout/termios (no injectable writer), so they are
//! `@pending` (needs real terminal teardown capture).

use super::*;
use quecto_tui::shell::app::tui_harness::TuiHarness;

fn with_harness<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    if world.tui_parity.is_none() {
        let rt = world.tui_parity_rt.as_ref().expect("runtime");
        let h = rt.block_on(async { TuiHarness::new().await });
        world.tui_parity = Some(TuiParityHarness(h));
    }
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    f(&mut world.tui_parity.as_mut().expect("TUI harness").0)
}

#[given("the TUI enabled modifyOtherKeys mode")]
fn given_enabled_modify_other_keys(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.enable_modify_other_keys();
        assert!(
            h.modify_other_keys_enabled(),
            "modifyOtherKeys should be active before exit"
        );
    });
}

#[when("the TUI exits")]
fn when_tui_exits(world: &mut TuiWorld) {
    // Run the exact protocol cleanup the event loop performs on teardown.
    with_harness(world, |h| h.run_protocol_cleanup());
}

#[then("modifyOtherKeys should be reset to mode 0")]
fn then_modify_other_keys_reset(world: &mut TuiWorld) {
    let enabled = with_harness(world, |h| h.modify_other_keys_enabled());
    assert!(
        !enabled,
        "on-exit cleanup must reset modifyOtherKeys to mode 0"
    );
}
