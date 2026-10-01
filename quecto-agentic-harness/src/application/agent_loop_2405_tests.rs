//! #2405: the context ceiling comes from the model's real window, less what
//! the reply needs beside the prompt, and a window the ceiling cannot use
//! as declared says so once.

use super::*;
use crate::application::catalogue::dto::ModelLimits;
use crate::application::catalogue::ports::ModelRuntime;
use crate::domain::catalogue::PromptLimit::{self, SharedWithRequest, WindowLessOutputCap};

fn agent(max_context_tokens: usize, max_tokens: u32, window: Option<usize>) -> AgentLoopImpl {
    let provider = Arc::new(MockProvider::new(vec![]));
    AgentLoopImpl::new(AgentLoopConfig {
        max_context_tokens,
        max_tokens,
        model_context_window: window,
        ..test_config(provider, Box::new(MockRegistry::new()))
    })
}

fn limits(
    max_output_tokens: Option<u32>,
    context_window: Option<usize>,
    prompt_limit: PromptLimit,
) -> ModelLimits {
    ModelLimits {
        max_output_tokens,
        context_window,
        prompt_limit,
    }
}

/// At startup the composition hands the loop the model's limits: an OpenAI
/// window keeps the fixed input limit, less the headroom.
#[test]
fn the_startup_ceiling_is_the_fixed_input_limit_less_the_headroom() {
    let agent = agent(300_000, 8_192, None).with_model_limits(limits(
        Some(128_000),
        Some(400_000),
        WindowLessOutputCap,
    ));
    assert_eq!(agent.effective_max_context_tokens(), 258_400);
    assert_eq!(
        agent.max_context_tokens(),
        258_400,
        "the reported ceiling agrees"
    );
}

#[test]
fn a_switch_recomputes_the_ceiling_for_each_kind_of_window() {
    let mut agent = agent(300_000, 8_192, None);
    agent.apply_model(
        "openai-api/gpt-5.3-codex".into(),
        limits(Some(128_000), Some(400_000), WindowLessOutputCap),
    );
    assert_eq!(agent.effective_max_context_tokens(), 258_400);

    // No declared cap: what a request asks for is the reserve.
    agent.apply_model(
        "openai-api/gpt-5.3-codex-spark".into(),
        limits(None, Some(128_000), WindowLessOutputCap),
    );
    assert_eq!(agent.effective_max_context_tokens(), 119_808);

    // #2405 review M2: a provider checking prompt + max_tokens reserves what
    // a request can ask for (twice 8k after a cut-off), not the 64k cap.
    agent.apply_model(
        "acme/local".into(),
        limits(Some(65_536), Some(131_072), SharedWithRequest),
    );
    assert_eq!(agent.effective_max_context_tokens(), 131_072 - 16_384);

    // A configured budget below the room wins.
    agent.apply_model(
        "openai-api/gpt-5.5".into(),
        limits(Some(128_000), Some(1_050_000), WindowLessOutputCap),
    );
    assert_eq!(agent.effective_max_context_tokens(), 300_000);

    // An unknown window falls back to the configured budget.
    agent.apply_model(
        "acme/unknown".into(),
        limits(Some(128_000), None, SharedWithRequest),
    );
    assert_eq!(agent.effective_max_context_tokens(), 300_000);
}

/// A writer the test subscriber logs into.
#[derive(Clone, Default)]
struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The `context_ceiling` lines logged while `run` drives a fresh loop. A
/// test running at the same time can leave a callsite's cached interest
/// stale for this subscriber (#1053): rebuilt first, and retried.
fn ceiling_log(run: impl Fn(&tokio::runtime::Runtime)) -> Vec<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    let mut lines = Vec::new();
    for _ in 0..5 {
        let captured = Captured::default();
        let sink = captured.0.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured)
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            run(&runtime);
        });
        let text = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
        lines = text
            .lines()
            .filter(|line| line.contains("context_ceiling"))
            .map(str::to_string)
            .collect();
        if !lines.is_empty() {
            break;
        }
    }
    lines
}

/// Two requests, a switch to the same model and a switch back: one line,
/// logged at the first request with the ceiling after the swarm cap.
#[test]
fn an_unknown_window_is_logged_once_with_the_ceiling_after_the_swarm_cap() {
    let lines = ceiling_log(|runtime| {
        let mut agent = agent(200_000, 8_192, None);
        agent.context_ceiling_cap().lower_to(48_000);
        let mut messages = vec![crate::domain::message::Message::user("go")];
        runtime.block_on(agent.apply_context_pruning(&mut messages, 1, false));
        runtime.block_on(agent.apply_context_pruning(&mut messages, 2, false));
        agent.apply_model("test-model".into(), limits(None, None, SharedWithRequest));
        agent.apply_model(
            "acme/known".into(),
            limits(None, Some(100_000), SharedWithRequest),
        );
        agent.apply_model("test-model".into(), limits(None, None, SharedWithRequest));
        runtime.block_on(agent.apply_context_pruning(&mut messages, 3, false));
    });
    assert_eq!(lines.len(), 1, "{lines:#?}");
    let line = &lines[0];
    assert!(line.contains("declares no context window"), "{line}");
    assert!(line.contains("model=test-model"), "{line}");
    assert!(line.contains("max_context_tokens=48000"), "{line}");
}

/// #2405 review M1: a declared cap that leaves the prompt under half the
/// window is warned once; under a fixed input limit the ceiling stays the
/// provider's limit less the headroom (final review L1): 2,000 x 0.95.
#[test]
fn a_reserve_past_the_floor_is_warned_once() {
    let lines = ceiling_log(|runtime| {
        let mut agent = agent(300_000, 8_192, None);
        let tight = limits(Some(128_000), Some(130_000), WindowLessOutputCap);
        agent.apply_model("acme/tight".into(), tight);
        assert_eq!(agent.effective_max_context_tokens(), 1_900);
        let mut messages = vec![crate::domain::message::Message::user("go")];
        runtime.block_on(agent.apply_context_pruning(&mut messages, 1, false));
        agent.apply_model("acme/tight".into(), tight);
        runtime.block_on(agent.apply_context_pruning(&mut messages, 2, false));
    });
    let warned: Vec<_> = lines.iter().filter(|line| line.contains("WARN")).collect();
    assert_eq!(warned.len(), 1, "{lines:#?}");
    assert!(warned[0].contains("model=acme/tight"), "{}", warned[0]);
    assert!(
        warned[0].contains("max_context_tokens=1900"),
        "{}",
        warned[0]
    );
}

/// The notes are kept per model and kind of note: a model noted for having
/// no window is still warned when it later carries a reserve past the floor.
#[test]
fn each_kind_of_note_is_logged_once_per_model() {
    let lines = ceiling_log(|runtime| {
        let mut agent = agent(300_000, 8_192, None);
        let mut messages = vec![crate::domain::message::Message::user("go")];
        runtime.block_on(agent.apply_context_pruning(&mut messages, 1, false));
        let tight = limits(Some(32_000), Some(32_000), SharedWithRequest);
        agent.apply_model("test-model".into(), tight);
        runtime.block_on(agent.apply_context_pruning(&mut messages, 2, false));
        agent.apply_model("test-model".into(), tight);
        runtime.block_on(agent.apply_context_pruning(&mut messages, 3, false));
    });
    assert_eq!(lines.len(), 2, "{lines:#?}");
    assert!(
        lines[0].contains("declares no context window"),
        "{}",
        lines[0]
    );
    assert!(lines[1].contains("WARN"), "{}", lines[1]);
    assert!(lines[1].contains("model=test-model"), "{}", lines[1]);
}
