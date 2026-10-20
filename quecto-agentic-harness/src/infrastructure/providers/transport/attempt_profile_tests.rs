use super::{Profile, Surface};
use crate::infrastructure::providers::transport::wire_dialect::TEST_WIRE;

/// #2210: only a streaming reply's steps are bounded, by the provider's own
/// bound; a whole non-streaming reply sends nothing until complete.
#[tokio::test(start_paused = true)]
async fn only_streaming_surfaces_bound_a_silent_step() {
    use crate::infrastructure::providers::transport::stream_idle::{STREAM_IDLE_LIMIT, StreamIdle};
    use std::time::Duration;
    let bound = StreamIdle::new(Duration::from_secs(5));
    for (surface, streams) in [
        (Surface::Chat, false),
        (Surface::Assembled, true),
        (Surface::Incremental, true),
    ] {
        let profile = Profile::new(&TEST_WIRE, surface, Default::default());
        assert_eq!(profile.idle.limit(), STREAM_IDLE_LIMIT);
        let profile = Profile::new(&TEST_WIRE, surface, bound);
        assert_eq!(profile.idle, bound);
        assert_eq!(profile.streams(), streams);
        let late = tokio::time::sleep(Duration::from_secs(6));
        assert_eq!(profile.within(late).await.is_ok(), !streams, "{streams}");
    }
}
