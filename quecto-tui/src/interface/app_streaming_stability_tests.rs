use super::app_event_loop::{StreamRenderCoalescer, StreamRenderDecision};
use super::*;

#[test]
fn bursty_token_updates_are_coalesced_to_frame_interval() {
    let start = tokio::time::Instant::now();
    let mut coalescer = StreamRenderCoalescer::default();

    assert_eq!(
        coalescer.record_token_update(start),
        StreamRenderDecision::RenderNow,
        "the first token in a burst should be presented promptly"
    );

    for offset_ms in [1, 5, 10, 20, 32] {
        assert_eq!(
            coalescer.record_token_update(start + Duration::from_millis(offset_ms)),
            StreamRenderDecision::DeferUntil(start + STREAM_RENDER_INTERVAL),
            "token at {offset_ms}ms should be batched into the next frame"
        );
    }

    assert!(
        coalescer.render_due(start + STREAM_RENDER_INTERVAL),
        "a deferred token burst should render when the frame deadline arrives"
    );
    assert!(
        !coalescer.render_due(start + STREAM_RENDER_INTERVAL + Duration::from_millis(1)),
        "the frame deadline should be consumed after one render"
    );
}

#[test]
fn input_or_resize_render_resets_stream_frame_deadline() {
    let start = tokio::time::Instant::now();
    let mut coalescer = StreamRenderCoalescer::default();

    assert_eq!(
        coalescer.record_token_update(start),
        StreamRenderDecision::RenderNow
    );
    assert_eq!(
        coalescer.record_token_update(start + Duration::from_millis(5)),
        StreamRenderDecision::DeferUntil(start + STREAM_RENDER_INTERVAL)
    );

    coalescer.note_immediate_render(start + Duration::from_millis(10));

    assert!(
        !coalescer.render_due(start + STREAM_RENDER_INTERVAL),
        "an input/resize-driven render should consume pending token paint work"
    );
    assert_eq!(
        coalescer.record_token_update(start + Duration::from_millis(15)),
        StreamRenderDecision::DeferUntil(
            start + Duration::from_millis(10) + STREAM_RENDER_INTERVAL
        ),
        "tokens after an immediate render should be limited from that render time"
    );
}

// ── Event-loop render-path integration (through the real App) ──────────
//
// These drive `App::render_stream_event` / `App::render_and_note` — the exact
// helpers every event-loop select arm calls — and count frames the App
// actually painted, so the coalescing WIRING (not just the state machine) is
// covered (#1011 review).

// `start_paused` freezes the tokio clock so the coalescing assertions are
// deterministic even on a loaded CI machine (a slow 40-iteration loop could
// otherwise straddle real 33ms frame boundaries).
#[tokio::test(start_paused = true)]
async fn token_burst_through_event_loop_helpers_coalesces_paints() {
    let mut h = tui_harness::TuiHarness::new().await;
    h.stream_event(Event::AgentStart);
    let start = h.rendered_frames();
    for _ in 0..40 {
        h.stream_event(Event::Token { token: "x".into() });
    }
    let painted = h.rendered_frames() - start;
    // With the tokio clock paused, all 40 tokens land at the same instant as
    // the `AgentStart` immediate render, so the coalescer rate-limits every
    // one of them (limited from that render time) and paints ZERO extra
    // frames, deferring the whole burst to the deadline arm. An exact bound
    // catches a partially broken coalescer that a loose `< 40` would let pass
    // (#1011 review).
    assert_eq!(
        painted, 0,
        "a same-instant 40-token burst after an immediate render must defer \
         all paints to the deadline arm; got {painted}"
    );
    // The burst leaves a deferred tail paint, which the deadline arm flushes.
    assert!(
        h.pending_stream_paint(),
        "burst should leave a deferred paint"
    );
    assert!(
        h.fire_deferred_stream_paint(),
        "the deferred-paint arm must flush the tail of the burst"
    );
    assert!(
        !h.pending_stream_paint(),
        "deadline consumed after one paint"
    );
}

#[tokio::test(start_paused = true)]
async fn keypress_render_paints_immediately_and_consumes_deferred_paint() {
    let mut h = tui_harness::TuiHarness::new().await;
    h.stream_event(Event::AgentStart);
    for _ in 0..10 {
        h.stream_event(Event::Token { token: "x".into() });
    }
    assert!(h.pending_stream_paint(), "burst should defer a paint");

    // A keypress-driven render (stdin select arm) paints NOW…
    let before = h.rendered_frames();
    h.press(super::keys::Key::Char('k'));
    h.immediate_render();
    assert_eq!(
        h.rendered_frames(),
        before + 1,
        "input must paint immediately"
    );
    // …and consumes the deferred token paint (the frame includes the tokens).
    assert!(
        !h.fire_deferred_stream_paint(),
        "an immediate render consumes the pending deferred paint"
    );
}

#[tokio::test(start_paused = true)]
async fn non_token_event_renders_immediately_mid_burst() {
    let mut h = tui_harness::TuiHarness::new().await;
    h.stream_event(Event::AgentStart);
    for _ in 0..10 {
        h.stream_event(Event::Token { token: "x".into() });
    }
    let before = h.rendered_frames();
    h.stream_event(Event::AgentEnd {
        message_refs: vec![],
    });
    assert_eq!(
        h.rendered_frames(),
        before + 1,
        "a non-token event (AgentEnd) must flush and paint immediately"
    );
    assert!(
        !h.fire_deferred_stream_paint(),
        "the final flush consumes any deferred token paint"
    );
}

#[tokio::test(start_paused = true)]
async fn resize_mid_burst_paints_full_frame_and_consumes_deferred_paint() {
    let mut h = tui_harness::TuiHarness::new().await;
    h.stream_event(Event::AgentStart);
    for _ in 0..10 {
        h.stream_event(Event::Token { token: "x".into() });
    }
    assert!(h.pending_stream_paint(), "burst should defer a paint");

    // The resize select arm does a full redraw NOW…
    let before = h.rendered_frames();
    h.resize_render();
    assert_eq!(
        h.rendered_frames(),
        before + 1,
        "a resize must repaint immediately"
    );
    // …and consumes the deferred token paint (the full frame includes the
    // accumulated tokens).
    assert!(
        !h.pending_stream_paint(),
        "a resize render consumes the pending deferred paint"
    );
    assert!(
        !h.fire_deferred_stream_paint(),
        "no deferred paint left to fire after a resize render"
    );
}

// Advance the (paused) tokio clock past the frame interval and verify the
// deadline arm's `render_due` semantics through elapsed time — i.e. the 33ms
// constant actually gates the deferred paint (#1011 review).
#[tokio::test(start_paused = true)]
async fn deferred_paint_fires_only_once_frame_interval_elapses() {
    let mut h = tui_harness::TuiHarness::new().await;
    h.stream_event(Event::AgentStart);
    h.stream_event(Event::Token { token: "x".into() }); // deferred (same instant)
    assert!(h.pending_stream_paint(), "token should defer a paint");

    // Before the frame deadline the arm must not paint.
    tokio::time::advance(super::STREAM_RENDER_INTERVAL / 2).await;
    assert!(
        !h.poll_deferred_stream_paint(),
        "the deferred paint must not fire before the frame interval elapses"
    );

    // At/after the deadline it fires exactly once.
    tokio::time::advance(super::STREAM_RENDER_INTERVAL).await;
    let before = h.rendered_frames();
    assert!(
        h.poll_deferred_stream_paint(),
        "the deferred paint fires once the frame interval has elapsed"
    );
    assert_eq!(h.rendered_frames(), before + 1);
    assert!(
        !h.pending_stream_paint(),
        "deadline consumed after one paint"
    );
}
