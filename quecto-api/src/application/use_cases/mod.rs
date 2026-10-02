pub mod abort;
pub mod clear_history;
pub mod follow_up;
pub mod get_state;
pub mod get_subagents;
pub mod health_check;
pub mod send_prompt;
pub mod set_effort;
pub mod set_model;
pub mod set_tool_policy;
pub mod steer;
pub mod sync_ledger;
pub mod tools;

#[cfg(test)]
pub(crate) mod test_support;

use crate::domain::error::ApiError;

/// A prompt, steer or follow-up carries text, images, or both (#2422).
/// Images are validated by `quecto_image`, the same rules and refusal text
/// as the agent's, so a refused message is answered here and never sent.
fn admit_message(
    message: &str,
    images: Vec<quecto_image::ImagePayload>,
) -> Result<Vec<quecto_image::ImageAttachment>, ApiError> {
    let images = quecto_image::validate_images(images)
        .map_err(|refusal| ApiError::InvalidRequest(refusal.to_string()))?;
    if message.is_empty() && images.is_empty() {
        return Err(ApiError::InvalidRequest("message must not be empty".into()));
    }
    Ok(images)
}
