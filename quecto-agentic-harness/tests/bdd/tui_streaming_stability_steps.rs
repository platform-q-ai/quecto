//! Step definitions for `tui_streaming_stability.feature` (#972).

use crate::{QuectoWorld, TuiParityHarness};
use cucumber::{given, then, when};
use quecto_tui::infrastructure::client::Event;
use quecto_tui::infrastructure::render::DiffRenderer;
use quecto_tui::interface::app::tui_harness::TuiHarness;
use quecto_tui::interface::component::Component;
use quecto_tui::interface::components::chat::Chat;
use quecto_tui::interface::keys::Key;
use quecto_tui::interface::utils::visible_width;

fn with_harness<R>(world: &mut QuectoWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    if world.tui_parity.is_none() {
        let rt = world.tui_parity_rt.as_ref().expect("runtime");
        let h = rt.block_on(async { TuiHarness::sized(48, 16).await });
        world.tui_parity = Some(TuiParityHarness(h));
    }
    f(&mut world.tui_parity.as_mut().expect("TUI harness").0)
}

#[given("the TUI is receiving a sustained high-throughput assistant response")]
fn given_sustained_high_throughput_response(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        h.stream_event(Event::AgentStart);
        for _ in 0..160 {
            h.stream_event(Event::Token { token: "x".into() });
        }
    });
}

#[when("the response continues for an extended period")]
fn when_response_continues(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        for _ in 0..160 {
            h.stream_event(Event::Token { token: "y".into() });
        }
        // Any deferred tail paint fires exactly as the event loop would.
        h.fire_deferred_stream_paint();
    });
}

#[then("the TUI presents a stable frame without stray cursor blocks")]
fn then_stable_frame_without_stray_cursor_blocks(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        let frame = h.full_frame();
        // Exactly ONE streaming indicator in the chat body: zero means the
        // intentional cursor was lost; more than one is precisely the
        // stray-cursor artifact class reported in #972. (The sub-agent panel's
        // selection marker also uses ▌, so count only right of the divider.)
        let indicators: usize = frame
            .lines()
            .map(|l| l.rsplit('│').next().unwrap_or(l).matches('▌').count())
            .sum();
        assert_eq!(
            indicators, 1,
            "exactly one streaming indicator should be visible in the chat body, found {indicators}: {frame:?}"
        );
        assert!(
            frame.contains("yy"),
            "the tail of the sustained stream should be present in the frame: {frame:?}"
        );
    });
}

#[given("the TUI is receiving a burst of assistant tokens")]
fn given_burst_of_assistant_tokens(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        h.stream_event(Event::AgentStart);
        for _ in 0..40 {
            h.stream_event(Event::Token { token: "z".into() });
        }
    });
}

#[when("the user provides input during the burst")]
fn when_user_input_during_burst(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        // Keys route through the stdin select arm, which paints IMMEDIATELY
        // (bypassing the coalescer) — assert that wiring, not just the frame.
        let before = h.rendered_frames();
        h.press(Key::Char('o'));
        h.immediate_render();
        h.press(Key::Char('k'));
        h.immediate_render();
        assert_eq!(
            h.rendered_frames(),
            before + 2,
            "each keypress must paint immediately even mid-burst"
        );
        h.stream_event(Event::Token { token: "!".into() });
    });
}

#[then("the user input is reflected promptly while the response continues")]
fn then_user_input_reflected_promptly(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        let frame = h.full_frame();
        assert!(
            frame.contains("ok"),
            "typed input should be visible: {frame:?}"
        );
        assert!(
            frame.contains('▌'),
            "assistant response should still be streaming: {frame:?}"
        );
    });
}

#[when("the burst is presented to the user")]
fn when_burst_presented(world: &mut QuectoWorld) {
    // Drive 40 tokens through the REAL event-loop render decision
    // (`App::render_stream_event`) and count frames the App actually painted.
    let renders = with_harness(world, |h| {
        let before = h.rendered_frames();
        h.stream_event(Event::AgentStart);
        let start = h.rendered_frames();
        for _ in 0..40 {
            h.stream_event(Event::Token { token: "z".into() });
        }
        h.fire_deferred_stream_paint();
        let _ = before;
        h.rendered_frames() - start
    });
    world.tui_idle_spinner_frame = Some(renders);
}

#[then("the streaming response remains visually smooth without distracting flicker")]
fn then_streaming_response_smooth(world: &mut QuectoWorld) {
    let renders = world
        .tui_idle_spinner_frame
        .expect("render count captured by When step");
    assert!(
        renders < 40,
        "a 40-token burst should coalesce to fewer painted frames; got {renders}"
    );
    assert!(
        renders >= 1,
        "the burst must still paint at least one frame; got {renders}"
    );
}

#[given("an assistant response is streaming near the right edge of the chat frame")]
fn given_streaming_near_chat_edge(world: &mut QuectoWorld) {
    let mut chat = Chat::new();
    chat.append_token("1234567890");
    world.tui_chat = Some(chat);
}

#[when("the TUI presents the streaming response near the chat frame edge")]
fn when_present_streaming_near_edge(world: &mut QuectoWorld) {
    let chat = world.tui_chat.as_mut().expect("chat from Given step");
    world.tui_viewport_after_stream = chat.render(10);
}

#[then("the streaming indicator remains inside the chat frame")]
fn then_indicator_inside_chat_frame(world: &mut QuectoWorld) {
    let line = world.tui_viewport_after_stream.join("");
    assert!(
        line.contains('▌'),
        "streaming indicator should be visible: {line:?}"
    );
    assert!(
        visible_width(&line) <= 10,
        "streaming indicator should stay inside the chat width: {line:?}"
    );
}

#[given("the terminal cursor is hidden while an assistant response streams")]
fn given_terminal_cursor_hidden(world: &mut QuectoWorld) {
    // Streaming is in progress in the app before the display recovery below.
    with_harness(world, |h| {
        h.stream_event(Event::AgentStart);
        h.stream_event(Event::Token {
            token: "streaming".into(),
        });
    });
}

#[when("the display recovers during the streaming response")]
fn when_display_recovers(world: &mut QuectoWorld) {
    let mut out = Vec::new();
    let mut renderer = DiffRenderer::new(&mut out);
    renderer
        .render(&["streaming ▌".to_string()], 32)
        .expect("full render");
    renderer
        .render(&["streaming more ▌".to_string()], 32)
        .expect("diff render");
    world.stdout = String::from_utf8(out).expect("utf8 render output");
}

#[then("the real terminal cursor stays hidden")]
fn then_real_terminal_cursor_stays_hidden(world: &mut QuectoWorld) {
    assert!(
        world.stdout.matches("\x1b[?25l").count() >= 2,
        "full and diff recovery renders should re-hide the terminal cursor: {:?}",
        world.stdout
    );
}

#[given("an assistant response is streaming")]
fn given_assistant_response_streaming(world: &mut QuectoWorld) {
    with_harness(world, |h| {
        h.event(Event::AgentStart);
        h.event(Event::Token {
            token: "streaming".into(),
        });
    });
}

#[when("the TUI presents the streaming response")]
fn when_tui_presents_streaming_response(world: &mut QuectoWorld) {
    // Capture the RAW frame (ANSI intact) so the editor's reverse-video
    // cursor escape is observable.
    let frame = with_harness(world, |h| {
        h.press(Key::Char('q'));
        h.full_frame_raw()
    });
    world.stdout = frame;
}

#[then("the editor cursor and assistant streaming indicator remain visible")]
fn then_intentional_cursors_remain_visible(world: &mut QuectoWorld) {
    assert!(
        world.stdout.contains("\x1b[7m"),
        "editor reverse-video cursor should remain visible in the raw frame: {:?}",
        world.stdout
    );
    assert!(
        world.stdout.contains('q'),
        "typed editor input should remain visible: {:?}",
        world.stdout
    );
    assert!(
        world.stdout.contains('▌'),
        "assistant streaming indicator should remain visible: {:?}",
        world.stdout
    );
}
