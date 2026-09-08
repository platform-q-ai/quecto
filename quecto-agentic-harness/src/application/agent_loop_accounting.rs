use super::*;

impl AgentLoopImpl {
    /// Usage receipts remain available even when a later request fails or is cancelled.
    pub fn take_unreported_usage(&mut self) -> UsageTotals {
        std::mem::take(&mut *self.unreported_usage.lock().unwrap())
    }

    pub fn take_request_observations(
        &self,
    ) -> Vec<crate::domain::request_observation::RequestObservation> {
        self.take_request_diagnostics().recent
    }

    pub fn take_request_diagnostics(
        &self,
    ) -> crate::domain::request_observation::RequestDiagnostics {
        std::mem::take(&mut *self.request_observations.lock().unwrap())
    }
    /// A record is removed only after durable acknowledgment. Cancellation leaves
    /// the original ID pending, including when a writer may already have committed.
    pub async fn flush_request_accounting(&self) -> Result<(), DomainError> {
        if let Some(accounting) = &self.request_accounting {
            loop {
                let record = self.accounting_outbox.lock().unwrap().first().cloned();
                let Some(record) = record else {
                    break;
                };
                accounting.record(&record).await?;
                self.accounting_outbox
                    .lock()
                    .unwrap()
                    .retain(|pending| pending.request_id != record.request_id);
            }
        }
        Ok(())
    }
    pub fn with_tool_admission(
        mut self,
        admission: Option<Arc<dyn crate::domain::tool::ToolExecutionAdmission>>,
    ) -> Self {
        self.tool_admission = admission;
        self
    }
    pub fn with_request_accounting(
        mut self,
        accounting: Option<Arc<dyn crate::domain::request_observation::RequestAccounting>>,
    ) -> Self {
        self.request_accounting = accounting;
        self
    }

    pub fn with_request_admission(
        mut self,
        admission: Option<Arc<dyn crate::domain::provider::RequestAdmission>>,
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
