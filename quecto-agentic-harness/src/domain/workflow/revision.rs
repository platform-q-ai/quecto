use super::WorkflowEngine;

impl WorkflowEngine {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1).max(1);
    }
}
