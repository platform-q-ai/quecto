//! Folder-aware session home scope vocabulary (#2001, D1).
//!
//! Pure domain values only: no filesystem, Git, UI, or process calls.
//! Opaque session identity remains orthogonal to home-scope metadata.

use super::error::DomainError;

/// Schema version for versioned home-scope metadata writers/readers.
pub const HOME_SCOPE_METADATA_VERSION_V1: u32 = 1;

/// Canonical execution directory identity as exact path text (no silent normalisation).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct CanonicalExecutionLocation(String);

impl CanonicalExecutionLocation {
    /// Construct from a non-empty canonical path string. Panics if empty — tests
    /// that need the error path use [`Self::try_from_canonical_path`].
    pub fn from_canonical_path(path: impl Into<String>) -> Self {
        Self::try_from_canonical_path(path).expect("canonical execution location must be non-empty")
    }

    /// Affirmative allowlist: non-empty path text only.
    pub fn try_from_canonical_path(path: impl Into<String>) -> Result<Self, DomainError> {
        let path = path.into();
        if path.is_empty() {
            return Err(DomainError::Session(
                "canonical execution location must be non-empty".to_string(),
            ));
        }
        Ok(Self(path))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CanonicalExecutionLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Display label for a repository grouping — not a stable identity fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct RepositoryLabel(String);

impl RepositoryLabel {
    /// Affirmative allowlist: non-empty after trim (whitespace-only rejected).
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DomainError::Session(
                "repository label must be non-empty".to_string(),
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Related worktree grouping facts discovered elsewhere; pure values only.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryWorktreeGrouping {
    repository_label: RepositoryLabel,
    execution_location: CanonicalExecutionLocation,
    member_locations: Vec<CanonicalExecutionLocation>,
}

impl RepositoryWorktreeGrouping {
    /// Group related worktrees while keeping the session's actual execution
    /// directory visibly distinguished from other members.
    pub fn related_worktrees(
        repository_label: RepositoryLabel,
        execution_location: CanonicalExecutionLocation,
        member_locations: Vec<CanonicalExecutionLocation>,
    ) -> Self {
        Self {
            repository_label,
            execution_location,
            member_locations,
        }
    }

    pub fn repository_label(&self) -> &RepositoryLabel {
        &self.repository_label
    }

    pub fn execution_location(&self) -> &CanonicalExecutionLocation {
        &self.execution_location
    }

    pub fn member_locations(&self) -> &[CanonicalExecutionLocation] {
        &self.member_locations
    }

    pub fn contains_location(&self, location: &CanonicalExecutionLocation) -> bool {
        self.member_locations.iter().any(|m| m == location)
    }
}

/// Session home scope: scoped to an execution location, or legacy-unscoped.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionHomeScope {
    /// Explicitly associated with a canonical execution location (and optional
    /// repository/worktree grouping facts).
    Scoped {
        execution_location: CanonicalExecutionLocation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repository_grouping: Option<RepositoryWorktreeGrouping>,
    },
    /// Pre-scope records and readers of missing scope metadata.
    LegacyUnscoped,
}

impl SessionHomeScope {
    pub fn scoped(
        execution_location: CanonicalExecutionLocation,
        repository_grouping: Option<RepositoryWorktreeGrouping>,
    ) -> Self {
        Self::Scoped {
            execution_location,
            repository_grouping,
        }
    }

    pub fn is_scoped(&self) -> bool {
        matches!(self, Self::Scoped { .. })
    }

    pub fn is_legacy_unscoped(&self) -> bool {
        matches!(self, Self::LegacyUnscoped)
    }

    pub fn execution_location(&self) -> Option<&CanonicalExecutionLocation> {
        match self {
            Self::Scoped {
                execution_location, ..
            } => Some(execution_location),
            Self::LegacyUnscoped => None,
        }
    }

    pub fn repository_grouping(&self) -> Option<&RepositoryWorktreeGrouping> {
        match self {
            Self::Scoped {
                repository_grouping,
                ..
            } => repository_grouping.as_ref(),
            Self::LegacyUnscoped => None,
        }
    }

    /// Same-scope means both scoped homes share equal execution locations.
    /// Legacy never silently matches a scoped home.
    pub fn same_scope_as(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Scoped {
                    execution_location: a,
                    ..
                },
                Self::Scoped {
                    execution_location: b,
                    ..
                },
            ) => a == b,
            _ => false,
        }
    }

    pub fn matches_execution_location(&self, location: &CanonicalExecutionLocation) -> bool {
        match self {
            Self::Scoped {
                execution_location, ..
            } => execution_location == location,
            Self::LegacyUnscoped => false,
        }
    }
}

/// Explicit cross-folder / resume disposition choices (#2001 contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResumeDisposition {
    SameScope,
    OpenOriginal,
    ForkCurrent,
    Locate,
    Cancel,
}

impl ResumeDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SameScope => "same_scope",
            Self::OpenOriginal => "open_original",
            Self::ForkCurrent => "fork_current",
            Self::Locate => "locate",
            Self::Cancel => "cancel",
        }
    }

    /// Affirmative wire allowlist only.
    pub fn parse(name: &str) -> Result<Self, DomainError> {
        match name {
            "same_scope" => Ok(Self::SameScope),
            "open_original" => Ok(Self::OpenOriginal),
            "fork_current" => Ok(Self::ForkCurrent),
            "locate" => Ok(Self::Locate),
            "cancel" => Ok(Self::Cancel),
            _ => Err(DomainError::Session(format!(
                "resume disposition not allowlisted: {name}"
            ))),
        }
    }

    pub fn resumes_in_place(self) -> bool {
        matches!(self, Self::SameScope)
    }

    pub fn launches_fresh_runtime(self) -> bool {
        matches!(self, Self::OpenOriginal)
    }

    pub fn imports_transcript_only(self) -> bool {
        matches!(self, Self::ForkCurrent)
    }

    pub fn requires_explicit_reassociation(self) -> bool {
        matches!(self, Self::Locate)
    }

    pub fn aborts_selection(self) -> bool {
        matches!(self, Self::Cancel)
    }
}

/// How a session became associated with a home scope — never guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssociationProvenance {
    CreatedWithScope,
    ExplicitUserAssociation,
    LocateReassociation,
    ForkIntoCurrentScope,
}

impl AssociationProvenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreatedWithScope => "created_with_scope",
            Self::ExplicitUserAssociation => "explicit_user_association",
            Self::LocateReassociation => "locate_reassociation",
            Self::ForkIntoCurrentScope => "fork_into_current_scope",
        }
    }

    pub fn parse(name: &str) -> Result<Self, DomainError> {
        match name {
            "created_with_scope" => Ok(Self::CreatedWithScope),
            "explicit_user_association" => Ok(Self::ExplicitUserAssociation),
            "locate_reassociation" => Ok(Self::LocateReassociation),
            "fork_into_current_scope" => Ok(Self::ForkIntoCurrentScope),
            _ => Err(DomainError::Session(format!(
                "association provenance not allowlisted: {name}"
            ))),
        }
    }

    /// All allowlisted provenances are explicit; none are guessed bindings.
    pub fn is_guessed(self) -> bool {
        false
    }
}

impl serde::Serialize for AssociationProvenance {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for AssociationProvenance {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Versioned, migration-compatible home-scope metadata persisted beside identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeScopeMetadata {
    schema_version: u32,
    scope: SessionHomeScope,
    provenance: Option<AssociationProvenance>,
}

impl HomeScopeMetadata {
    pub fn v1_scoped(
        execution_location: CanonicalExecutionLocation,
        repository_grouping: Option<RepositoryWorktreeGrouping>,
        provenance: AssociationProvenance,
    ) -> Self {
        Self {
            schema_version: HOME_SCOPE_METADATA_VERSION_V1,
            scope: SessionHomeScope::scoped(execution_location, repository_grouping),
            provenance: Some(provenance),
        }
    }

    pub fn v1_legacy_unscoped() -> Self {
        Self {
            schema_version: HOME_SCOPE_METADATA_VERSION_V1,
            scope: SessionHomeScope::LegacyUnscoped,
            provenance: None,
        }
    }

    /// Absent optional JSON block → legacy-unscoped readable record.
    pub fn from_optional_json(raw: Option<&str>) -> Result<Self, DomainError> {
        match raw {
            None => Ok(Self::v1_legacy_unscoped()),
            Some(s) if s.trim().is_empty() => Ok(Self::v1_legacy_unscoped()),
            Some(s) => serde_json::from_str(s).map_err(|e| {
                DomainError::Session(format!("home scope metadata JSON: {e}"))
            }),
        }
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn scope(&self) -> &SessionHomeScope {
        &self.scope
    }

    pub fn provenance(&self) -> AssociationProvenance {
        self.provenance
            .expect("scoped metadata always carries explicit provenance")
    }

    pub fn provenance_opt(&self) -> Option<AssociationProvenance> {
        self.provenance
    }

    pub fn is_legacy_unscoped(&self) -> bool {
        self.scope.is_legacy_unscoped()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct HomeScopeMetadataWire {
    #[serde(default = "default_schema_v1")]
    schema_version: u32,
    #[serde(default)]
    execution_location: Option<String>,
    #[serde(default)]
    repository_grouping: Option<RepositoryWorktreeGrouping>,
    #[serde(default)]
    provenance: Option<AssociationProvenance>,
    /// Optional legacy tag some writers may emit; ignored when location present.
    #[serde(default)]
    scope: Option<String>,
}

fn default_schema_v1() -> u32 {
    HOME_SCOPE_METADATA_VERSION_V1
}

impl serde::Serialize for HomeScopeMetadata {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = match &self.scope {
            SessionHomeScope::Scoped {
                execution_location,
                repository_grouping,
            } => HomeScopeMetadataWire {
                schema_version: self.schema_version,
                execution_location: Some(execution_location.as_str().to_string()),
                repository_grouping: repository_grouping.clone(),
                provenance: self.provenance,
                scope: None,
            },
            SessionHomeScope::LegacyUnscoped => HomeScopeMetadataWire {
                schema_version: self.schema_version,
                execution_location: None,
                repository_grouping: None,
                provenance: None,
                scope: Some("legacy_unscoped".to_string()),
            },
        };
        wire.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for HomeScopeMetadata {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = HomeScopeMetadataWire::deserialize(deserializer)?;
        if wire.schema_version != HOME_SCOPE_METADATA_VERSION_V1 {
            return Err(serde::de::Error::custom(format!(
                "unsupported home scope metadata schema version {}",
                wire.schema_version
            )));
        }
        let scope = match wire.execution_location {
            Some(path) if !path.is_empty() => {
                let loc = CanonicalExecutionLocation::try_from_canonical_path(path)
                    .map_err(serde::de::Error::custom)?;
                SessionHomeScope::scoped(loc, wire.repository_grouping)
            }
            _ => SessionHomeScope::LegacyUnscoped,
        };
        let provenance = match &scope {
            SessionHomeScope::Scoped { .. } => Some(
                wire.provenance
                    .unwrap_or(AssociationProvenance::ExplicitUserAssociation),
            ),
            SessionHomeScope::LegacyUnscoped => None,
        };
        let _ = wire.scope; // optional tag from legacy writers
        Ok(Self {
            schema_version: wire.schema_version,
            scope,
            provenance,
        })
    }
}

#[cfg(test)]
#[path = "session_home_scope_tests.rs"]
mod tests;
