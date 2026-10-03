//! #2435 review round 1 M2: on the real render path, a model the harness
//! says cannot run is marked in the selector with why, and choosing it sends
//! nothing.

use super::tui_harness::*;
use crate::shell::keys::Key;

fn models() -> serde_json::Value {
    serde_json::json!([
        {"model": "openai-oauth/gpt-5.5-mini", "provider": "openai-oauth", "auth": "oauth",
         "configured": false,
         "unavailable": ["refused-for-account: not supported with a ChatGPT account"]},
        {"model": "openai-oauth/gpt-6.1-sol", "provider": "openai-oauth", "auth": "oauth",
         "configured": true, "unavailable": []},
    ])
}

#[tokio::test]
async fn an_unavailable_model_is_marked_and_cannot_be_chosen() {
    let mut h = TuiHarness::new().await;
    h.request_model_selector_open();
    h.deliver_list_models_json(models());
    h.capture();
    let frame = h.last();
    assert!(
        frame.contains("(unavailable: refused-for-account: not supported with a ChatGPT"),
        "the row says why: {frame}"
    );
    let _ = h.try_drain_commands();
    h.press(Key::Enter);
    let sent = h.try_drain_commands();
    assert!(
        sent.iter().all(|line| !line.contains("set_model")),
        "no switch is sent for an unavailable model: {sent:?}"
    );
    h.capture();
    assert!(
        h.last().contains("is unavailable"),
        "the selector says why it stayed open: {}",
        h.last()
    );
    h.press(Key::Down);
    h.press(Key::Enter);
    let sent = h.drain_commands().await;
    assert!(
        sent.iter()
            .any(|line| line.contains("set_model") && line.contains("gpt-6.1-sol")),
        "a runnable model is still chosen: {sent:?}"
    );
}
