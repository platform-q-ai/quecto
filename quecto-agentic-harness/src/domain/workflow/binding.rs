use super::WorkflowEngine;

impl WorkflowEngine {
    /// Bind the engine to its currently-selected template (a by-value
    /// `--workflow-spec` assignment). Once bound, the model cannot switch or
    /// re-select a different template. See [`WorkflowEngine`].
    pub fn set_bound(&mut self, bound: bool) {
        self.bound = bound;
    }

    /// Whether this engine is bound to a single assigned template.
    pub fn is_bound(&self) -> bool {
        self.bound
    }
}
