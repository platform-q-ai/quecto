use super::*;

impl AgentLoopImpl {
    /// Usage receipts remain available even when a later request fails or is cancelled.
    pub fn take_unreported_usage(&mut self) -> UsageTotals {
        std::mem::take(
            &mut *self
                .unreported_usage
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    /// A run stopped where it stood (a `--max-time` deadline, #2168): write
    /// the requests it left unfinished to the audit log as `cancelled`, say
    /// why the run stopped, and flush the accounting outbox as a finished
    /// turn would. The records are written with turn 0: after the run.
    pub async fn settle_stopped_run(&self, reason: &str) -> Result<(), DomainError> {
        let unfinished: Vec<_> = self
            .request_observations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recent
            .iter()
            .filter(|record| record.outcome == "cancelled")
            .cloned()
            .collect();
        for record in unfinished {
            self.audit(
                0,
                AuditEvent::RequestObserved {
                    observation: Box::new(record),
                },
            )
            .await;
        }
        self.audit(
            0,
            AuditEvent::Error {
                source: "deadline".into(),
                tool: None,
                message: reason.into(),
            },
        )
        .await;
        self.flush_request_accounting().await
    }

    pub fn take_request_observations(
        &self,
    ) -> Vec<crate::domain::request_observation::RequestObservation> {
        self.take_request_diagnostics().recent
    }

    pub fn take_request_diagnostics(
        &self,
    ) -> crate::domain::request_observation::RequestDiagnostics {
        std::mem::take(
            &mut *self
                .request_observations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
    /// A record is removed only after durable acknowledgment. Cancellation leaves
    /// the original ID pending, including when a writer may already have committed.
    /// A full diagnostic ledger drops the record with a warning: diagnostics
    /// must never block inference, including the coordinator's final report.
    pub async fn flush_request_accounting(&self) -> Result<(), DomainError> {
        if let Some(accounting) = &self.request_accounting {
            loop {
                let record = self
                    .accounting_outbox
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .first()
                    .cloned();
                let Some(record) = record else {
                    break;
                };
                match accounting.record(&record).await {
                    Ok(()) => {}
                    Err(error) => match classify_accounting_failure(&error) {
                        // Contention: keep the record pending and retry next turn.
                        AccountingFailure::Transient => return Err(error),
                        // Diagnostics never block inference: a durable rejection
                        // (ledger full, revoked member, ...) drops the record
                        // with an audited warning.
                        AccountingFailure::Durable => tracing::warn!(
                            request_id = %record.request_id,
                            %error,
                            "request diagnostic dropped after a durable rejection; export the run to retain diagnostics"
                        ),
                    },
                }
                self.accounting_outbox
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .retain(|pending| pending.request_id != record.request_id);
            }
        }
        Ok(())
    }
    pub fn with_tool_admission(
        mut self,
        admission: Option<Arc<dyn crate::application::tools::ports::ToolExecutionAdmission>>,
    ) -> Self {
        self.tool_admission = admission;
        self
    }
    pub fn with_request_accounting(
        mut self,
        accounting: Option<Arc<dyn crate::application::providers::ports::RequestAccounting>>,
    ) -> Self {
        self.request_accounting = accounting;
        self
    }

    pub fn with_request_admission(
        mut self,
        admission: Option<Arc<dyn crate::application::providers::ports::RequestAdmission>>,
    ) -> Self {
        self.request_admission = admission;
        self
    }
}

impl AgentLoop for AgentLoopImpl {
    fn process<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<AgentResult, DomainError>> + Send + 'a>>
    {
        Box::pin(self.run_loop(messages))
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            tool_count: self.tool_catalog().tool_count(),
        }
    }
}

impl AgentLoopImpl {
    pub fn with_max_tool_iterations(mut self, max: u32) -> Self {
        self.max_tool_iterations = max;
        self
    }

    #[cfg(test)]
    pub fn with_progress_callback(mut self, callback: Option<ProgressCallback>) -> Self {
        self.progress_callback = callback;
        self
    }
}

/// How a store rejection of a diagnostic record is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountingFailure {
    /// The store was busy (SQLite lock contention): retry on the next turn.
    Transient,
    /// A durable refusal (ledger full, unknown member, ...): drop the record.
    Durable,
}

/// The bridge reports SQLite contention as `... is locked`; everything else the
/// store refuses is durable for this process.
pub(crate) fn classify_accounting_failure(error: &DomainError) -> AccountingFailure {
    if error.to_string().to_ascii_lowercase().contains("is locked") {
        AccountingFailure::Transient
    } else {
        AccountingFailure::Durable
    }
}

#[cfg(test)]
#[path = "agent_loop_accounting_tests.rs"]
mod accounting_tests;
