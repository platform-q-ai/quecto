//! Pure tool vocabulary: definitions, results and runtime policy values.
//!
//! The tool ports (`Tool`, `ToolGuard`, `ToolCatalog`, `ToolExecutor`,
//! `ToolPolicyMutator`, `RuntimeToolLifecycleRegistry`, `SessionAwareTools`,
//! `ToolRegistry`, `ToolExecutionAdmission`) are the application's
//! (`application::tools::ports`, #1960).
use std::borrow::Cow;

use super::conversation::stored_images::{ImageDigest, VerifiedText, sha256_hex};
use super::tool_descriptor::{ProfileAvailabilityScope, ToolAvailability, ToolCatalogueEntry};

/// Metadata describing a tool for the LLM.
///
/// Fields use `Cow<'static, str>` so that static tool schemas (the common
/// case — 11 of 12 tools) are zero-cost clones (pointer copy), while
/// dynamic schemas (`ls` with runtime limits) use `Cow::Owned`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: Cow<'static, str>,
    pub description: Cow<'static, str>,
    /// JSON Schema string describing the parameters.
    pub parameters_schema: Cow<'static, str>,
}

/// An image a message carries (#2423): a tool's result (e.g. `read` on an
/// image file, or an extension's `imageBlocks`) or a user's attachment
/// (`UserImageBlock`, #2422). Its fields are private, so a block is made
/// only from an admitted image ([`quecto_image::ImageAttachment`], the
/// normal path) or by [`ImageBlock::restore`], which re-admits what
/// persistence kept: the type is the allowlist for every image a provider
/// is sent. It carries the admitted type and keeps it; no step after
/// admission can lose or forge it. `quecto_image` is a pure leaf crate (no
/// I/O, depends only on base64 and serde), so the domain depends on it as
/// it does on `base64`.
#[derive(Clone)]
pub struct ImageBlock {
    mime: quecto_image::ImageMime,
    /// Strict standard base64. Private, as is its digest (#2424): a
    /// block's text never changes.
    data: String,
    /// The SHA-256 of `data`, once asked for (#2424: what a session stores it as).
    digest: ImageDigest,
}

/// Blocks are equal when their images are: the cached digest is not part of it.
impl PartialEq for ImageBlock {
    fn eq(&self, other: &Self) -> bool {
        (self.mime, &self.data) == (other.mime, &other.data)
    }
}

impl Eq for ImageBlock {}

/// The normal way to make a block: from an admitted image.
impl From<quecto_image::ImageAttachment> for ImageBlock {
    fn from(image: quecto_image::ImageAttachment) -> Self {
        Self {
            mime: image.mime(),
            data: image.into_data(),
            digest: ImageDigest::default(),
        }
    }
}

impl ImageBlock {
    /// A block persistence kept (#2424 saves them): its type and base64
    /// are re-admitted by the strict rules, never trusted, so a corrupt or
    /// edited session file cannot carry an image admission would refuse.
    pub fn restore(
        mime: quecto_image::ImageMime,
        data: String,
    ) -> Result<Self, quecto_image::ImageRefusal> {
        let payload = quecto_image::ImagePayload::new(mime.as_str(), data);
        quecto_image::ImageAttachment::new(payload).map(Self::from)
    }

    /// [`Self::restore`] for a sidecar's text, hashed when it was read
    /// (#2424), for either kind of image (#2423): kept only when admission
    /// leaves it exactly as read, with the digest known rather than computed
    /// again; `None` for one admission refuses (the session keeps it as an
    /// unloaded reference). Admission hands the text back unchanged (the
    /// same allocation), so the check costs no copy; were it ever to
    /// rewrite it, the text is hashed once to tell.
    pub(crate) fn restore_verified(
        mime: quecto_image::ImageMime,
        text: VerifiedText,
    ) -> Option<Self> {
        let (sha256, data) = text.into_parts();
        let (at, len) = (data.as_ptr(), data.len());
        let block = Self::restore(mime, data).ok()?;
        let unchanged = std::ptr::eq(block.data.as_ptr(), at) && block.data.len() == len;
        let same = unchanged || sha256_hex(block.data.as_bytes()) == sha256;
        same.then(|| Self {
            digest: ImageDigest::verified(sha256),
            ..block
        })
    }

    /// The admitted type.
    pub fn mime(&self) -> quecto_image::ImageMime {
        self.mime
    }

    /// The admitted type as the wire spells it.
    pub fn mime_type(&self) -> &'static str {
        self.mime.as_str()
    }

    /// The image's strict standard base64.
    pub fn data(&self) -> &str {
        &self.data
    }

    /// The SHA-256 of the block's text, computed once (#2424).
    pub fn sha256(&self) -> &str {
        self.digest.of(&self.data)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn digest_builds_for_tests(&self) -> usize {
        self.digest.builds_for_tests()
    }

    /// A block whose `data` is not admitted, for tests of what is done
    /// with a block (serializers, markers) that need short, known data.
    /// Compiled only for tests: production blocks come from admission.
    #[cfg(any(test, feature = "test-support"))]
    pub fn unchecked_for_tests(mime: quecto_image::ImageMime, data: impl Into<String>) -> Self {
        Self {
            mime,
            data: data.into(),
            digest: ImageDigest::default(),
        }
    }

    /// A 1x1 admitted image of `mime`, for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn sample(mime: quecto_image::ImageMime) -> Self {
        let bytes = quecto_image::samples::sample(mime);
        quecto_image::ImageAttachment::from_bytes(mime, &bytes)
            .expect("a sample is admitted")
            .into()
    }
}

/// Never prints the base64.
impl std::fmt::Debug for ImageBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageBlock")
            .field("mime", &self.mime)
            .field("data_len", &self.data.len())
            .finish()
    }
}

/// The result of executing a tool.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    /// Optional image blocks (e.g. when `read` is called on an image file,
    /// or an extension's `tool_result` carries `imageBlocks`). Empty for
    /// all non-image tools — zero-cost default.
    pub image_blocks: Vec<ImageBlock>,
    /// Internal delivery metadata passed to `Tool::result_delivered` after
    /// `content` has been appended. This is never surfaced to the model.
    pub delivery_metadata: Option<String>,
}

impl ToolResult {
    /// A tool call that returned an error, as the model sees it (#2247
    /// round 2 N4): `Error: <error>`, an error result with nothing else.
    pub fn from_error(error: &impl std::fmt::Display) -> Self {
        Self {
            content: format!("Error: {error}"),
            is_error: true,
            image_blocks: Vec::new(),
            delivery_metadata: None,
        }
    }
}

/// Which profile a tool catalogue is being read for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProfileContext {
    Parent,
    Child,
}

/// Requested runtime policy state for a registered tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPolicyMutation {
    pub name: String,
    pub availability: ToolAvailability,
    pub scope: ProfileAvailabilityScope,
    pub reason: String,
}

impl ToolPolicyMutation {
    pub fn enable(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            availability: ToolAvailability::Enabled,
            scope: ProfileAvailabilityScope::Both,
            reason: reason.into(),
        }
    }

    pub fn disable(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            availability: ToolAvailability::Disabled,
            scope: ProfileAvailabilityScope::None,
            reason: reason.into(),
        }
    }

    pub fn set_scope(
        name: impl Into<String>,
        scope: ProfileAvailabilityScope,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            availability: ToolAvailability::from(scope),
            scope,
            reason: reason.into(),
        }
    }
}

impl From<ProfileAvailabilityScope> for ToolAvailability {
    fn from(scope: ProfileAvailabilityScope) -> Self {
        if matches!(scope, ProfileAvailabilityScope::None) {
            Self::Disabled
        } else {
            Self::Enabled
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicyApplyMode {
    ImmediateIfIdle,
    AtNextTurnBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicyOperation {
    Patch,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPolicyRequest {
    pub operation: ToolPolicyOperation,
    pub mutations: Vec<ToolPolicyMutation>,
    pub unlisted_scope: Option<ProfileAvailabilityScope>,
    pub correlation_id: Option<String>,
    pub persist: bool,
}

impl ToolPolicyRequest {
    pub fn patch(mutations: Vec<ToolPolicyMutation>) -> Self {
        Self {
            operation: ToolPolicyOperation::Patch,
            mutations,
            unlisted_scope: None,
            correlation_id: None,
            persist: false,
        }
    }

    pub fn replace(
        mutations: Vec<ToolPolicyMutation>,
        unlisted_scope: ProfileAvailabilityScope,
    ) -> Self {
        Self {
            operation: ToolPolicyOperation::Replace,
            mutations,
            unlisted_scope: Some(unlisted_scope),
            correlation_id: None,
            persist: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicyMutationStatus {
    Applied,
    AlreadyInState,
    UnknownTool,
    BlockedByRestriction,
    PersistenceFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicyMutationResult {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_identifier: Option<String>,
    pub requested_availability: ToolAvailability,
    pub requested_scope: ProfileAvailabilityScope,
    pub status: ToolPolicyMutationStatus,
    pub before: Option<ToolCatalogueEntry>,
    pub after: Option<ToolCatalogueEntry>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicyReconciliation {
    pub mode: ToolPolicyApplyMode,
    pub results: Vec<ToolPolicyMutationResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;

impl ToolDefinition {
    /// What sending this definition with a request costs: its name,
    /// description and parameter schema (#2160).
    pub fn estimated_tokens(&self) -> usize {
        use crate::domain::message::Message;
        Message::estimate_tokens(&self.name)
            + Message::estimate_tokens(&self.description)
            + Message::estimate_tokens(&self.parameters_schema)
    }
}
