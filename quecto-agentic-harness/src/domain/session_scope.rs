//! Pure vocabulary for a session's home scope.
//! No value in this module performs filesystem, Git, process, or UI work.

use serde::{Deserialize, Serialize};

pub const CURRENT_SCOPE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonicalExecutionLocation(String);

impl CanonicalExecutionLocation {
    pub fn new(path: impl Into<String>) -> Result<Self, &'static str> {
        let path = path.into();
        if path.is_empty() {
            return Err("canonical execution location must not be empty");
        }
        Ok(Self(path))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryGrouping {
    worktree_git_dir: String,
    common_git_dir: String,
}

impl RepositoryGrouping {
    pub fn new(
        worktree_git_dir: impl Into<String>,
        common_git_dir: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let worktree_git_dir = worktree_git_dir.into();
        let common_git_dir = common_git_dir.into();
        if worktree_git_dir.is_empty() || common_git_dir.is_empty() {
            return Err("repository grouping facts must not be empty");
        }
        Ok(Self {
            worktree_git_dir,
            common_git_dir,
        })
    }

    pub fn worktree_git_dir(&self) -> &str { &self.worktree_git_dir }
    pub fn common_git_dir(&self) -> &str { &self.common_git_dir }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssociationProvenance {
    Discovered,
    ExplicitlyAssociated,
    ExplicitlyLocated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionHomeScope {
    LegacyUnscoped,
    Scoped {
        execution_location: CanonicalExecutionLocation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repository_grouping: Option<RepositoryGrouping>,
        provenance: AssociationProvenance,
    },
}

impl SessionHomeScope {
    pub fn scoped(
        execution_location: CanonicalExecutionLocation,
        repository_grouping: Option<RepositoryGrouping>,
        provenance: AssociationProvenance,
    ) -> Self {
        Self::Scoped { execution_location, repository_grouping, provenance }
    }

    pub fn execution_location(&self) -> Option<&CanonicalExecutionLocation> {
        match self { Self::Scoped { execution_location, .. } => Some(execution_location), Self::LegacyUnscoped => None }
    }
    pub fn repository_grouping(&self) -> Option<&RepositoryGrouping> {
        match self { Self::Scoped { repository_grouping, .. } => repository_grouping.as_ref(), Self::LegacyUnscoped => None }
    }
    pub fn provenance(&self) -> Option<AssociationProvenance> {
        match self { Self::Scoped { provenance, .. } => Some(*provenance), Self::LegacyUnscoped => None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionScopeMetadata {
    schema_version: u16,
    home: SessionHomeScope,
}

impl SessionScopeMetadata {
    pub fn current(home: SessionHomeScope) -> Self {
        Self { schema_version: CURRENT_SCOPE_SCHEMA_VERSION, home }
    }
    pub fn schema_version(&self) -> u16 { self.schema_version }
    pub fn home(&self) -> &SessionHomeScope { &self.home }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeDisposition {
    SameScope,
    OpenOriginal,
    ForkCurrent,
    Locate,
    Cancel,
}
