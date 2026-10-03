//! #2421: the built-in vision models declare `image` input, so the gate sends
//! them images; a text-only model declares text alone.

use super::ModelRegistry;

/// The built-in providers whose every model takes images, but the ones in
/// [`TEXT_ONLY`].
const VISION_PROVIDERS: [&str; 5] = [
    "anthropic-api",
    "anthropic-oauth",
    "openai-api",
    "openai-oauth",
    "xai",
];

/// GPT-5.3 Codex Spark takes text only (OpenAI, at launch).
const TEXT_ONLY: [&str; 1] = ["gpt-5.3-codex-spark"];

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
        .filter(|record| !TEXT_ONLY.contains(&record.id.as_str()))
        .collect();
    assert!(vision.len() >= 20, "the built-in vision models are listed");
    for record in vision {
        assert!(
            declares(&record.input, "image") && declares(&record.input, "text"),
            "{} declares {:?}",
            record.qualified_id(),
            record.input
        );
    }
}

#[test]
fn codex_spark_declares_text_input_only() {
    let registry = ModelRegistry::builtin();
    for provider in ["openai-api", "openai-oauth"] {
        let spark = registry
            .find(provider, "gpt-5.3-codex-spark")
            .expect("spark is built in");
        assert_eq!(spark.input, vec!["text".to_string()]);
    }
}
