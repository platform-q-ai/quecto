//! The conversation a model is sent, as pure policy over its messages.

pub mod image_input;
pub mod image_tokens;
pub mod user_images;
pub mod watermark;
pub mod watermark_cut;

/// What a user-role message is to the watermark context (#2403), stamped
/// when it is appended and saved with it, so a cut pins the latest prompt
/// and never a harness message that came after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UserKind {
    /// Not a prompt: the harness's feedback, a sub-agent's note, a swarm
    /// wake or a workflow nudge; and every message saved before #2403 (an
    /// old session's prompts plan as the harness's: no legacy effort).
    #[default]
    Unmarked,
    /// A prompt: what the user sent, or the task a parent or a swarm gave
    /// a sub-agent or a member.
    Prompt,
    /// The stub a watermark cut put in place of what it archived.
    ArchiveStub,
}
