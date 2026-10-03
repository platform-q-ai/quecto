//! The built-in models #2435 retired: GPT models older than GPT-5.6 and
//! Claude models older than Claude 5, by the built-in provider that listed
//! them. A configured model among them is named as retired, so a warning
//! can say why it is no longer listed; any other unlisted id is not.

/// The retired built-ins, by the built-in providers that listed them.
const RETIRED: &[(&[&str], &[&str])] = &[
    (
        &["openai-api", "openai-oauth"],
        &[
            "gpt-5.5",
            "gpt-5.5-mini",
            "gpt-5.5-nano",
            "gpt-5.3-codex",
            "gpt-5.3-codex-spark",
            "gpt-5.2-codex",
        ],
    ),
    (
        &["anthropic-api", "anthropic-oauth"],
        &[
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-opus-4-5",
            "claude-sonnet-4-6",
            "claude-sonnet-4-5",
        ],
    ),
];

/// Whether `model` was a built-in of `provider` before #2435 retired it,
/// both matched whatever their case.
pub fn retired_builtin(provider: &str, model: &str) -> bool {
    RETIRED.iter().any(|(providers, models)| {
        providers
            .iter()
            .any(|listed| listed.eq_ignore_ascii_case(provider))
            && models
                .iter()
                .any(|retired| retired.eq_ignore_ascii_case(model))
    })
}

#[cfg(test)]
#[path = "retired_tests.rs"]
mod tests;
