// CodingJobService trait implementation for CodingCoordinator.
//
// Extracted to a separate file to keep coding_coordinator.rs within the
// 750-line quality gate limit. This module is `#[path = ...]` included
// from coding_coordinator.rs and shares its scope.

use crate::domain::coding_command::{
    CancelResponse, CleanupResponse, CommandError, CreateRequest, CreateResponse, ImportRequest,
    ImportResponse, ListRequest, ListResponse, RunRequest, RunResponse, StatusResponse,
};
use crate::domain::coding_ports::{CodingJobService, RepoValidator, SkillResolver};

use super::CodingCoordinator;

impl<R: RepoValidator + Send, S: SkillResolver + Send> CodingJobService
    for CodingCoordinator<R, S>
{
    fn create_repo(&mut self, _req: CreateRequest) -> Result<CreateResponse, CommandError> {
        // The coordinator manages jobs, not repositories. Repo creation is
        // handled by DriverJobService which holds a RepoCreator.
        Err(CommandError::Internal(
            "create_repo not supported on bare coordinator".to_string(),
        ))
    }

    fn import_repo(&mut self, _req: ImportRequest) -> Result<ImportResponse, CommandError> {
        Err(CommandError::Internal(
            "import_repo not supported on bare coordinator".to_string(),
        ))
    }

    fn run(&mut self, req: RunRequest) -> Result<RunResponse, CommandError> {
        CodingCoordinator::run(self, req)
    }

    fn status_by_job_id(&self, job_id: &str) -> Result<StatusResponse, CommandError> {
        CodingCoordinator::status_by_job_id(self, job_id)
    }

    fn status_by_run_id(&self, run_id: &str) -> Result<StatusResponse, CommandError> {
        CodingCoordinator::status_by_run_id(self, run_id)
    }

    fn cancel(&mut self, job_id: &str) -> Result<CancelResponse, CommandError> {
        CodingCoordinator::cancel(self, job_id)
    }

    fn cleanup(
        &mut self,
        job_id: &str,
        keep_artifacts: bool,
    ) -> Result<CleanupResponse, CommandError> {
        CodingCoordinator::cleanup(self, job_id, keep_artifacts)
    }

    fn list(&self, req: &ListRequest) -> ListResponse {
        CodingCoordinator::list(self, req)
    }
}
