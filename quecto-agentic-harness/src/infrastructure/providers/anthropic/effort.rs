//! Anthropic's effort vocabulary (#1066), relocated from `mod.rs` for the
//! 750-line cap.

/// Map an effort level onto Anthropic's documented vocabulary
/// (`low`/`medium`/`high`/`max`). The OpenAI-only levels (#1066) clamp to
/// the nearest documented Anthropic value; Anthropic's own levels are
/// transmitted verbatim, unchanged from the pre-#1066 behaviour.
pub(super) fn anthropic_effort_str(
    effort: crate::domain::inference::value_objects::provider::EffortLevel,
) -> &'static str {
    use crate::domain::inference::value_objects::provider::EffortLevel;
    match effort {
        EffortLevel::None => "low",
        EffortLevel::XHigh => "high",
        other => other.as_str(),
    }
}
