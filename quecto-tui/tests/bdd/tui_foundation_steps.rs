//! Steps for the two `@tui @done` scenarios in `tui_foundation.feature`:
//! command-send and render failures must be *observable*.
//!
//! Both drive REAL production failure paths:
//!  * a disconnected `Client` + real `send_command` → the real
//!    `handle_command_send_failure` surfaces an error notification, and
//!  * a `DiffRenderer` over a writer that returns `io::Error` → the real
//!    `render` propagates the error, and `handle_render_failure` surfaces it.
//!
//! The other foundation scenarios remain `@pending` and are untouched.

use super::*;
use quecto_tui::protocol::client::Command;
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::render::DiffRenderer;
use std::io::{self, Write};

/// A writer whose every write/flush fails — models a broken terminal fd.
struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("simulated terminal write failure"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("simulated terminal flush failure"))
    }
}

fn run_harness<R>(f: impl FnOnce(&mut TuiHarness) -> R + Send) -> R
where
    R: Send,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async {
        let mut h = TuiHarness::new().await;
        f(&mut h)
    })
}

// ── Command send failures are observable ──────────────────────────────────

#[given("the TUI command channel is disconnected")]
fn given_channel_disconnected(world: &mut TuiWorld) {
    // The real disconnect happens in the When via `send_command_expecting_failure`
    // (it swaps in a disconnected client); flag the intent here.
    world.tui_foundation_disconnect = true;
}

#[when("the TUI tries to send a command to the agent")]
fn when_send_command(world: &mut TuiWorld) {
    assert!(
        world.tui_foundation_disconnect,
        "the channel should have been marked disconnected"
    );
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let notification = rt.block_on(async {
        let mut h = TuiHarness::new().await;
        h.send_command_expecting_failure(Command::GetState {
            id: Some("bdd".to_string()),
        })
        .await
    });
    world.tui_foundation_notification = notification;
}

#[then("the TUI should show an error notification for the failed command send")]
fn then_command_error_notification(world: &mut TuiWorld) {
    assert!(
        world
            .tui_foundation_notification
            .contains("Failed to send get_state command"),
        "a command-send failure must surface as an error notification, got: {}",
        world.tui_foundation_notification
    );
}

#[then("the send failure should not be handled only through stderr")]
fn then_not_only_stderr(world: &mut TuiWorld) {
    assert!(
        !world.tui_foundation_notification.trim().is_empty(),
        "the failure must be visible in the UI notification stack, not just stderr"
    );
}

// ── DiffRenderer write and flush failures are observable ───────────────────

#[given("the TUI renderer output fails while writing or flushing")]
fn given_renderer_fails(world: &mut TuiWorld) {
    world.tui_foundation_render_was_err = false;
}

#[when("the TUI renders a frame")]
fn when_render_frame(world: &mut TuiWorld) {
    // The real DiffRenderer over a failing writer must return the error.
    let mut renderer = DiffRenderer::new(FailingWriter);
    let err = renderer
        .render(&["alpha".to_string(), "beta".to_string()], 80)
        .expect_err("render over a failing writer must return an error");
    world.tui_foundation_render_was_err = true;
    // Feed that real error through the production render-failure handler and
    // capture the notification it raises.
    world.tui_foundation_notification = run_harness(move |h| h.handle_render_failure(&err));
}

#[then("the DiffRenderer should return the render error instead of ignoring it")]
fn then_render_returns_error(world: &mut TuiWorld) {
    assert!(
        world.tui_foundation_render_was_err,
        "DiffRenderer::render must propagate the write/flush error"
    );
}

#[then("the TUI should show an error notification for the failed render")]
fn then_render_error_notification(world: &mut TuiWorld) {
    assert!(
        world
            .tui_foundation_notification
            .contains("Failed to render frame"),
        "a render failure must surface as an error notification, got: {}",
        world.tui_foundation_notification
    );
}
