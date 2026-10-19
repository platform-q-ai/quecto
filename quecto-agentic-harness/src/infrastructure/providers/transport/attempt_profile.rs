//! Existing wire/error behavior is a property of a provider and call surface,
//! not admission policy. Keep these compatibility decisions out of ownership.
use super::stream_idle::{BodyError, Idle, StreamIdle};
use super::wire_dialect::WireDialect;
use crate::domain::shared::error::DomainError;

#[derive(Clone, Copy)]
pub(in crate::infrastructure::providers) enum Surface {
    Chat,
    Assembled,
    Incremental,
}
#[derive(Clone, Copy)]
pub(in crate::infrastructure::providers) struct Profile {
    /// What the provider's wire means (#2597).
    pub dialect: &'static dyn WireDialect,
    pub surface: Surface,
    /// The provider's bound on a silent stream (#2210).
    pub idle: StreamIdle,
}
impl Profile {
    /// A profile bound by `idle`: always the provider's own bounds, which
    /// every call site must name (#2210 review), so none can fall back to
    /// the defaults and ignore a provider's chosen bounds.
    pub fn new(dialect: &'static dyn WireDialect, surface: Surface, idle: StreamIdle) -> Self {
        Self {
            dialect,
            surface,
            idle,
        }
    }
    /// Whether the reply streams, so its send and every read of its body are
    /// bounded by [`Self::idle`]. A whole non-streaming reply sends nothing
    /// until it is complete: there the bound would be a total one.
    pub fn streams(self) -> bool {
        matches!(self.surface, Surface::Assembled | Surface::Incremental)
    }
    /// Await one step of the exchange: bounded when the reply streams.
    pub async fn within<F: std::future::Future>(self, step: F) -> Result<F::Output, Idle> {
        match self.streams() {
            true => self.idle.within(step).await,
            false => Ok(step.await),
        }
    }
    /// The idle bound of this reply's SSE body, measured from its last event
    /// (#2433). Only a streaming reply has an SSE body read this way.
    pub fn event_idle(self) -> super::stream_idle::EventIdle {
        assert!(self.streams(), "only a streaming reply has an SSE body");
        super::stream_idle::EventIdle::events(self.idle)
    }
    /// Read a whole body: each read bounded when the reply streams.
    pub async fn text(self, response: reqwest::Response) -> Result<String, BodyError> {
        match self.streams() {
            true => self.idle.text(response).await,
            false => response.text().await.map_err(BodyError::Read),
        }
    }
    pub fn name(self) -> &'static str {
        self.dialect.name()
    }
    pub fn send_error(self, error: &reqwest::Error) -> DomainError {
        DomainError::Provider(self.dialect.send_error(error))
    }
    pub fn read_error(self, error: &reqwest::Error) -> DomainError {
        let label = self.dialect.read_label(self.surface);
        DomainError::Provider(format!("failed to read {label}: {error}"))
    }
    pub fn strict_error_body(self) -> bool {
        self.dialect.strict_error_body(self.surface)
    }
    pub fn suffix(self, headers: &reqwest::header::HeaderMap) -> String {
        self.dialect.retry_after(headers)
    }
    pub fn error_body(self, body: String) -> String {
        if matches!(self.surface, Surface::Incremental) {
            super::sse_common::truncate_error_body(body)
        } else {
            body
        }
    }
}

#[cfg(test)]
#[path = "attempt_profile_tests.rs"]
mod tests;
