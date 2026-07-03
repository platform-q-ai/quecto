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
