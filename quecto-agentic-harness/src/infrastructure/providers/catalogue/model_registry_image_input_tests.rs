//! #2421: the built-in vision models declare `image` input, so the gate sends
//! them images; a text-only model declares text alone.

use super::ModelRegistry;

/// The built-in providers whose every model takes images.
const VISION_PROVIDERS: [&str; 5] = [
    "anthropic-api",
    "anthropic-oauth",
    "openai-api",
    "openai-oauth",
    "xai",
];

fn declares(input: &[String], modality: &str) -> bool {
    input.iter().any(|declared| declared == modality)
}

#[test]
fn every_builtin_claude_gpt_and_grok_model_declares_image_input() {
    let registry = ModelRegistry::builtin();
    let vision: Vec<_> = registry
        .models()
        .iter()
        .filter(|record| VISION_PROVIDERS.contains(&record.provider.as_str()))
        .collect();
    assert!(vision.len() >= 20, "the built-in vision models are listed");
    assert_eq!(
        vision.len(),
        registry.models().len(),
        "every built-in model takes images"
    );
    for record in vision {
        assert!(
            declares(&record.input, "image") && declares(&record.input, "text"),
            "{} declares {:?}",
            record.qualified_id(),
            record.input
        );
    }
}

/// A model the built-in tables do not list (one `models.json` declares
/// without `input`) takes text only.
#[test]
fn a_model_outside_the_builtin_tables_declares_text_input_only() {
    assert_eq!(
        super::builtin_input("acme", "seeing"),
        vec!["text".to_string()]
    );
    assert_eq!(
        super::builtin_input("anthropic-api", "claude-opus-4-8"),
        vec!["text".to_string()]
    );
}
