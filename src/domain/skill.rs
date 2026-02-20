use super::error::DomainError;

/// A loaded skill that extends agent capabilities.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    /// The skill's content (typically from SKILL.md).
    pub content: String,
    /// Where this skill was loaded from.
    pub source: SkillSource,
}

/// The origin of a loaded skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillSource {
    Workspace,
    Global,
    Builtin,
}

/// Port: loads skills from various sources.
pub trait SkillLoader: Send + Sync {
    /// List all available skills across all sources.
    fn list(&self) -> Result<Vec<Skill>, DomainError>;

    /// Load a specific skill by name.
    fn load(&self, name: &str) -> Result<Option<Skill>, DomainError>;
}
