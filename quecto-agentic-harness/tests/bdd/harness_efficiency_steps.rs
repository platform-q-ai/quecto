//! Step definitions for `harness_efficiency.feature` (issue #996).
//!
//! Each scenario is self-contained: state lives on the shared World fields
//! declared for these scenarios, so no new World wiring is required.

use super::*;
use quecto::domain::audit::content_preview;

#[when("a 500-character multibyte string is previewed to 100 characters")]
fn when_preview_multibyte(world: &mut QuectoWorld) {
    // 500 two-byte 'é' characters — a mid-codepoint cut would corrupt the
    // output or panic when slicing on a non-boundary byte index.
    let input = "é".repeat(500);
    world.efficiency_preview = Some(content_preview(&input, 100));
}

#[then("the preview shows 100 characters ending in an ellipsis")]
fn then_preview_bounded(world: &mut QuectoWorld) {
    let out = world.efficiency_preview.as_ref().expect("preview computed");
    assert_eq!(
        out.chars().count(),
        100,
        "preview must be bounded to 100 chars"
    );
    assert!(
        out.ends_with("..."),
        "truncated preview must end with ellipsis"
    );
}

#[then("every previewed character is a whole codepoint")]
fn then_preview_utf8(world: &mut QuectoWorld) {
    let out = world.efficiency_preview.as_ref().expect("preview computed");
    // The only real UTF-8-safety proof: the kept prefix must be exactly 97
    // intact 'é' characters followed by the "..." ellipsis. A mid-codepoint
    // cut could not reconstruct to whole 'é' chars.
    let expected = format!("{}...", "é".repeat(97));
    assert_eq!(
        *out, expected,
        "preview must cut on a char boundary, not mid-codepoint"
    );
}

#[when("an OpenAI response reports 12 prompt, 7 completion and 19 total tokens")]
fn when_parse_openai_usage(world: &mut QuectoWorld) {
    use quecto::infrastructure::providers::usage;
    let v = serde_json::json!({
        "prompt_tokens": 12, "completion_tokens": 7, "total_tokens": 19
    });
    world.efficiency_usage = Some(usage::parse_openai_usage(v.as_object().unwrap()));
}

#[then("the recorded usage shows 12 prompt, 7 completion and 19 context tokens")]
fn then_openai_usage(world: &mut QuectoWorld) {
    let u = world.efficiency_usage.as_ref().expect("usage parsed");
    assert_eq!(u.prompt_tokens, 12);
    assert_eq!(u.completion_tokens, 7);
    assert_eq!(u.context_tokens, Some(19));
}

#[when("a Codex response reports 100 input, 40 output and 30 cached tokens")]
fn when_parse_codex_usage(world: &mut QuectoWorld) {
    use quecto::infrastructure::providers::usage;
    let v = serde_json::json!({
        "input_tokens": 100, "output_tokens": 40,
        "input_tokens_details": { "cached_tokens": 30 }
    });
    world.efficiency_usage = Some(usage::parse_codex_usage(v.as_object().unwrap()));
}

#[then("the recorded usage shows 100 prompt, 40 completion and 30 cached tokens")]
fn then_codex_usage(world: &mut QuectoWorld) {
    let u = world.efficiency_usage.as_ref().expect("usage parsed");
    assert_eq!(u.prompt_tokens, 100);
    assert_eq!(u.completion_tokens, 40);
    assert_eq!(u.cache_read_tokens, Some(30));
}

#[when("a provider config written by an older release is loaded")]
fn when_load_legacy_config(world: &mut QuectoWorld) {
    use quecto::infrastructure::config::ProviderEntry;
    // A blob from before the dead `auth_method` field was removed. Serde
    // ignores the now-unknown key, so old on-disk configs still load.
    let json =
        r#"{ "api_key": "sk-x", "api_base": "https://example.test", "auth_method": "api_key" }"#;
    let entry: ProviderEntry = serde_json::from_str(json).expect("legacy config must load");
    world.efficiency_provider_entry = Some(entry);
}

#[then("the config loads and its api_key and api_base are read back")]
fn then_config_loaded(world: &mut QuectoWorld) {
    use quecto::infrastructure::config::ProviderEntry;
    let entry = world
        .efficiency_provider_entry
        .as_ref()
        .expect("config loaded");
    assert_eq!(entry.api_key, "sk-x");
    assert_eq!(entry.api_base, "https://example.test");
    // The absence of any `auth_method` field is enforced at compile time by
    // this exhaustive struct literal — if the field were reintroduced this
    // would fail to build.
    let _explicit = ProviderEntry {
        api_key: "k".into(),
        api_base: "b".into(),
        disable_codex_routing: false,
    };
}
