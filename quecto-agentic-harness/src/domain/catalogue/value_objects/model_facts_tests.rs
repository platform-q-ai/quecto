//! #2405: the ceiling from the real window (`ModelWindow`).

use super::*;
use PromptLimit::{SharedWithRequest, WindowLessOutputCap};

fn window(
    window: Option<usize>,
    limit: PromptLimit,
    cap: Option<usize>,
    requested: usize,
) -> ModelWindow {
    ModelWindow::new(window, limit, cap, requested)
}

/// The provider's fixed input limit, less the headroom.
fn fixed_limit(window: usize, cap: usize) -> usize {
    let limit = window.saturating_sub(cap);
    limit - limit * FIXED_INPUT_HEADROOM_PERCENT / 100
}

/// OpenAI fixes the input limit at the window less the output cap (400k =
/// 272k + 128k), less the headroom Codex also keeps (95% of 272k).
#[test]
fn an_openai_window_leaves_the_fixed_input_limit_less_the_headroom() {
    let codex = window(Some(400_000), WindowLessOutputCap, Some(128_000), 8_192);
    assert_eq!(FIXED_INPUT_HEADROOM_PERCENT, 5);
    assert_eq!(codex.prompt_room(), Some(258_400));
    assert_eq!(codex.ceiling(300_000), 258_400);
    // A configured budget below the room wins.
    assert_eq!(codex.ceiling(200_000), 200_000);
    let api = window(Some(1_050_000), WindowLessOutputCap, Some(128_000), 8_192);
    assert_eq!(api.prompt_room(), Some(875_900));
    // Without a declared cap the provider's input limit is unknown: the
    // reply's reserve is what a request asks for, with no headroom.
    let spark = window(Some(128_000), WindowLessOutputCap, None, 8_192);
    assert_eq!(spark.prompt_room(), Some(119_808));
}

/// #2405 final review L1: the half-window floor must not lift a fixed
/// input limit: with a 200k window and a 128k cap the provider takes 72k,
/// so the room is 68,400, not 100,000.
#[test]
fn a_fixed_input_ceiling_never_exceeds_the_providers_limit_less_the_headroom() {
    let caps = [0, 1, 8_192, 94_000, 128_000, 199_999, 200_000, 1_000_000];
    let mut windows: Vec<usize> = (1..=600_000).step_by(997).collect();
    windows.extend([128_000, 130_000, 200_000, 256_000, 400_000]);
    for cap in caps {
        for &w in &windows {
            let fixed = window(Some(w), WindowLessOutputCap, Some(cap), 8_192);
            let room = fixed.prompt_room().expect("a known window has a room");
            assert_eq!(room, fixed_limit(w, cap), "cap {cap} window {w}");
            assert!(fixed.ceiling(usize::MAX) <= fixed_limit(w, cap));
        }
    }
    let tight = window(Some(200_000), WindowLessOutputCap, Some(128_000), 8_192);
    assert_eq!(tight.prompt_room(), Some(68_400));
    assert!(
        tight.reserve_clamped(),
        "a limit under half the window is warned"
    );
}

/// Elsewhere the provider checks the prompt plus the requested output, so
/// the reserve is what a request can ask for: the effective limit, or up
/// to twice it after an output-limit cut-off (#2124), never the full cap.
#[test]
fn a_shared_window_reserves_what_a_request_can_ask_for() {
    // A user model, 128k window and a 64k cap, asking for 8k a request.
    let user = window(Some(131_072), SharedWithRequest, Some(65_536), 8_192);
    assert_eq!(user.prompt_room(), Some(131_072 - 16_384));
    // A cap under twice the request bounds the raised limit.
    let low_cap = window(Some(131_072), SharedWithRequest, Some(10_000), 8_192);
    assert_eq!(low_cap.prompt_room(), Some(121_072));
    // No declared cap: no raise, the request itself.
    let local = window(Some(32_768), SharedWithRequest, None, 1_024);
    assert_eq!(local.ceiling(300_000), 31_744);
    // An unknown window falls back to the configured budget.
    let unknown = window(None, WindowLessOutputCap, Some(128_000), 8_192);
    assert_eq!(unknown.ceiling(256_000), 256_000);
    assert_eq!(
        window(None, SharedWithRequest, None, 8_192).prompt_room(),
        None
    );
    assert_eq!(ModelWindow::default().ceiling(256_000), 256_000);
}

/// #2405 review M1: the ceiling never falls as the window grows, whatever
/// the cap or the kind. Where the provider shares the window with the
/// request (or the input limit is unknown), the prompt also keeps at least
/// half the window: a cap just under the window no longer collapses it.
#[test]
fn the_ceiling_is_monotonic_in_the_window_and_a_shared_one_never_collapses() {
    let caps = [
        None,
        Some(0),
        Some(1),
        Some(8_192),
        Some(65_536),
        Some(128_000),
        Some(1_000_000),
    ];
    let mut windows: Vec<usize> = (1..=600_000).step_by(997).collect();
    for edge in [128_000usize, 131_072, 256_000] {
        windows.extend(edge - 3..=edge + 3);
    }
    windows.sort_unstable();
    for limit in [SharedWithRequest, WindowLessOutputCap] {
        for cap in caps {
            for requested in [0, 1_024, 8_192, 128_000] {
                let mut previous = 0;
                for &w in &windows {
                    let model = window(Some(w), limit, cap, requested);
                    let room = model.prompt_room().expect("a known window has a room");
                    assert!(
                        room >= previous,
                        "{limit:?} cap {cap:?} request {requested}: {previous} at a smaller window, {room} at {w}"
                    );
                    assert!(room <= w, "{limit:?} cap {cap:?}: {room} in {w}");
                    let floored = limit == SharedWithRequest || cap.is_none();
                    if floored {
                        assert!(room >= w / 2, "{limit:?} cap {cap:?}: {room} in {w}");
                    }
                    previous = room;
                }
            }
        }
    }
    // The reviewer's case: no collapse just past a reserve of 128k.
    for w in [128_000, 128_001, 130_000] {
        let room = window(Some(w), SharedWithRequest, Some(128_000), 128_000).prompt_room();
        assert!(room >= Some(w / 2), "{w}: {room:?}");
    }
}

/// When the reserve would leave the prompt under half the window it is
/// reported, so the caller can warn: a shared reserve is clamped (the room
/// is half the window), a fixed input limit is kept as the provider sets it.
#[test]
fn a_reserve_past_the_floor_is_reported() {
    let tight = window(Some(130_000), WindowLessOutputCap, Some(128_000), 8_192);
    assert_eq!(tight.prompt_room(), Some(1_900));
    assert!(tight.reserve_clamped());
    // A cap that fills a shared window still leaves the reply room.
    let full = window(Some(32_000), SharedWithRequest, Some(32_000), 32_000);
    assert_eq!(full.prompt_room(), Some(16_000));
    assert!(full.reserve_clamped());
    let roomy = window(Some(400_000), WindowLessOutputCap, Some(128_000), 8_192);
    assert!(!roomy.reserve_clamped());
    assert!(!window(None, SharedWithRequest, None, 8_192).reserve_clamped());
}

/// The whole-window budget ignores the reserve: a raised output limit may
/// use what the prompt leaves of the window (#2124).
#[test]
fn the_whole_window_budget_ignores_the_reply_reserve() {
    let codex = window(Some(400_000), WindowLessOutputCap, Some(128_000), 8_192);
    assert_eq!(codex.budget(300_000), 300_000);
    let small = window(Some(100_000), SharedWithRequest, Some(128_000), 8_192);
    assert_eq!(small.budget(300_000), 100_000);
    assert_eq!(
        window(None, SharedWithRequest, None, 8_192).budget(300_000),
        300_000
    );
}
