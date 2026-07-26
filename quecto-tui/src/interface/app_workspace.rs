use std::path::PathBuf;

use crate::interface::components::files_autocomplete::FilesAutocomplete;

pub(super) struct WorkspaceFlow {
    pub(super) files_autocomplete: FilesAutocomplete,
    /// Last observed git branch shown in the footer.
    pub(super) git_branch: Option<String>,
    /// Repository root used for git branch polling.
    pub(super) git_repo: Option<PathBuf>,
}

impl WorkspaceFlow {
    pub(super) fn new(git_branch: Option<String>, git_repo: Option<PathBuf>) -> Self {
        Self {
            files_autocomplete: FilesAutocomplete::new(8),
            git_branch,
            git_repo,
        }
    }
}
