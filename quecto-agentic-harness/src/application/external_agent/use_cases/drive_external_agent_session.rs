//! Drive one claude-code member's session (#2287). RED stub.

use std::sync::Arc;

use crate::application::external_agent::dto::{
    AbortOutcome, ExternalAgentSessionSettings, FinalReport, ProjectedMessage, PromptAccepted,
    SessionRefusal, SessionStep, SessionView, StreamingBehavior,
};
use crate::application::external_agent::ports::{ExternalAgentLauncher, ExternalAgentTelemetry};

/// The member's session.
pub struct DriveExternalAgentSession {
    launcher: Arc<dyn ExternalAgentLauncher>,
    telemetry: Arc<dyn ExternalAgentTelemetry>,
    settings: ExternalAgentSessionSettings,
}

impl DriveExternalAgentSession {
    pub fn new(
        launcher: Arc<dyn ExternalAgentLauncher>,
        telemetry: Arc<dyn ExternalAgentTelemetry>,
        settings: ExternalAgentSessionSettings,
    ) -> Self {
        Self {
            launcher,
            telemetry,
            settings,
        }
    }

    pub async fn start(&self) -> Result<(), SessionRefusal> {
        let _ = (&self.launcher, &self.telemetry, &self.settings);
        Ok(())
    }

    pub async fn prompt(
        &self,
        text: &str,
        behavior: Option<StreamingBehavior>,
    ) -> Result<PromptAccepted, SessionRefusal> {
        let _ = (text, behavior);
        Err(SessionRefusal::NotStarted)
    }

    pub async fn steer(&self, text: &str) -> Result<PromptAccepted, SessionRefusal> {
        self.prompt(text, Some(StreamingBehavior::Steer)).await
    }

    pub async fn follow_up(&self, text: &str) -> Result<PromptAccepted, SessionRefusal> {
        self.prompt(text, Some(StreamingBehavior::FollowUp)).await
    }

    pub async fn abort(&self) -> Result<AbortOutcome, SessionRefusal> {
        Err(SessionRefusal::NotStarted)
    }

    pub async fn next_step(&self) -> Option<SessionStep> {
        None
    }

    pub fn state(&self) -> SessionView {
        SessionView {
            phase: Default::default(),
            execution: Default::default(),
            queued_follow_ups: 0,
            totals: Default::default(),
        }
    }

    pub fn report(&self) -> Option<FinalReport> {
        None
    }

    pub fn messages(&self, start: usize, count: usize) -> Vec<ProjectedMessage> {
        let _ = (start, count);
        Vec::new()
    }
}

#[cfg(test)]
#[path = "drive_external_agent_session_tests.rs"]
mod tests;
