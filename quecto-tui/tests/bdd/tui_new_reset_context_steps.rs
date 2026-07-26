//! Steps for `tui_new_reset_context.feature`.
//!
//! Drives the REAL `/new` and `/clear` slash-command handlers through the
//! headless render harness (via `handle_submit`) and asserts the footer's
//! context gauge, populated by a real `TurnEnd` usage report, resets to the
//! unknown `?/0` form.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::interface::app::tui_harness::TuiHarness;
use quecto_tui::protocol::client::Event;

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

/// Populate the footer context gauge via a real TurnEnd usage report.
fn set_context_usage(world: &mut TuiWorld, context_tokens: u64, window: u64) {
    with_harness(world, |h| {
        h.event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant",
                "content": "",
                "contextTokens": context_tokens,
                "maxContextTokens": window,
            }),
        });
    });
    let footer = with_harness(world, |h| h.bottom_stack());
    assert!(
        !footer.contains("?/0"),
        "precondition: the footer should show populated context usage, got:\n{footer}"
    );
}

// ── Given ──────────────────────────────────────────────────────────────────

#[given(regex = r#"^the footer shows "45\.2%/200k" context usage$"#)]
fn footer_shows_specific(world: &mut TuiWorld) {
    // 90,400 / 200,000 = 45.2%.
    set_context_usage(world, 90_400, 200_000);
    let footer = with_harness(world, |h| h.bottom_stack());
    assert!(
        footer.contains("45.2%") && footer.contains("200k"),
        "precondition: footer should show 45.2%/200k, got:\n{footer}"
    );
}

#[given("the footer shows context usage data")]
fn footer_shows_data(world: &mut TuiWorld) {
    set_context_usage(world, 60_000, 200_000);
}

// ── When ───────────────────────────────────────────────────────────────────

#[when("the user executes /new")]
fn executes_new(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.submit("/new");
    });
}

#[when("the user executes /clear")]
fn executes_clear(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.submit("/clear");
    });
}

// ── Then ───────────────────────────────────────────────────────────────────

#[then(regex = r#"^the footer context should reset to "\?/0"$"#)]
fn footer_resets_to_zero(world: &mut TuiWorld) {
    let footer = with_harness(world, |h| h.bottom_stack());
    assert!(
        footer.contains("?/0"),
        "footer context should reset to ?/0, got:\n{footer}"
    );
    assert!(
        !footer.contains('%'),
        "a reset footer should show no context percentage, got:\n{footer}"
    );
}

#[then("the footer context should reset")]
fn footer_resets(world: &mut TuiWorld) {
    let footer = with_harness(world, |h| h.bottom_stack());
    assert!(
        footer.contains("?/0"),
        "footer context should reset to the unknown ?/0 form, got:\n{footer}"
    );
}
