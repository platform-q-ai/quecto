use std::collections::HashMap;

use super::error::DomainError;

/// A loaded skill that extends agent capabilities.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    /// Short description from SKILL.md frontmatter (1–1024 chars).
    pub description: String,
    /// The skill's body content (everything after the closing `---`).
    pub content: String,
    /// Where this skill was loaded from.
    pub source: SkillSource,
}

/// The origin of a loaded skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillSource {
    Workspace,
}

/// YAML frontmatter extracted from a SKILL.md file.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub compatibility: Option<String>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, String>>,
}

/// Port: loads skills from various sources.
pub trait SkillLoader: Send + Sync {
    /// List all available skills across all sources.
    fn list(&self) -> Result<Vec<Skill>, DomainError>;

    /// Load a specific skill by name.
    fn load(&self, name: &str) -> Result<Option<Skill>, DomainError>;
}

/// Validate a skill name: lowercase alphanumeric with single hyphen
/// separators, 1–64 characters. Regex: `^[a-z0-9]+(-[a-z0-9]+)*$`
pub fn is_valid_skill_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    if name.starts_with('-') || name.ends_with('-') {
        return false;
    }
    if name.contains("--") {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Parse a SKILL.md file into its frontmatter and body.
///
/// Expects the file to start with `---`, followed by YAML, then a
/// closing `---`. Everything after the closing delimiter is the body.
/// Returns `None` if the file does not contain valid frontmatter.
pub fn parse_skill_md(raw: &str) -> Option<(SkillFrontmatter, String)> {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    // Find the closing --- (skip the opening one)
    let after_open = &trimmed[3..];
    let close_idx = find_closing_delimiter(after_open)?;
    let yaml_block = &after_open[..close_idx];
    let body_start = close_idx + 3; // skip the closing "---"
    let body = if body_start < after_open.len() {
        after_open[body_start..].trim().to_string()
    } else {
        String::new()
    };

    let fm: SkillFrontmatter = serde_yaml::from_str(yaml_block).ok()?;

    // Validate required fields are non-empty
    if fm.name.is_empty() || fm.description.is_empty() {
        return None;
    }
    if fm.description.len() > 1024 {
        return None;
    }

    Some((fm, body))
}

/// Find the position of the closing `---` delimiter.
/// Looks for `\n---` (newline followed by three dashes) to avoid
/// matching `---` inside YAML values.
fn find_closing_delimiter(s: &str) -> Option<usize> {
    // The closing delimiter must be on its own line
    for (i, _) in s.match_indices("\n---") {
        // Check that what follows is either EOF, newline, or whitespace
        let after = i + 4; // skip "\n---"
        if after >= s.len() {
            return Some(i + 1); // +1 to skip the \n
        }
        let next_char = s.as_bytes()[after];
        if next_char == b'\n' || next_char == b'\r' || next_char == b' ' {
            return Some(i + 1);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_skill_names() {
        assert!(is_valid_skill_name("weather"));
        assert!(is_valid_skill_name("code-review"));
        assert!(is_valid_skill_name("git-release"));
        assert!(is_valid_skill_name("a"));
        assert!(is_valid_skill_name("a1b2"));
        assert!(is_valid_skill_name("my-cool-skill"));
    }

    #[test]
    fn test_invalid_skill_names() {
        assert!(!is_valid_skill_name(""));
        assert!(!is_valid_skill_name("-bad"));
        assert!(!is_valid_skill_name("bad-"));
        assert!(!is_valid_skill_name("bad--name"));
        assert!(!is_valid_skill_name("My_Skill"));
        assert!(!is_valid_skill_name("UPPERCASE"));
        assert!(!is_valid_skill_name("has space"));
        assert!(!is_valid_skill_name("has_underscore"));
        let long = "a".repeat(65);
        assert!(!is_valid_skill_name(&long));
    }

    #[test]
    fn test_max_length_name_accepted() {
        let name = "a".repeat(64);
        assert!(is_valid_skill_name(&name));
    }

    #[test]
    fn test_parse_basic_frontmatter() {
        let raw = "---\nname: weather\ndescription: Fetch weather\n---\nBody content";
        let (fm, body) = parse_skill_md(raw).unwrap();
        assert_eq!(fm.name, "weather");
        assert_eq!(fm.description, "Fetch weather");
        assert_eq!(body, "Body content");
        assert!(fm.license.is_none());
        assert!(fm.metadata.is_none());
    }

    #[test]
    fn test_parse_all_optional_fields() {
        let raw = "\
---
name: git-release
description: Create releases
license: MIT
compatibility: opencode
metadata:
  audience: maintainers
  workflow: github
---
## Steps
- Draft notes";
        let (fm, body) = parse_skill_md(raw).unwrap();
        assert_eq!(fm.name, "git-release");
        assert_eq!(fm.license.unwrap(), "MIT");
        assert_eq!(fm.compatibility.unwrap(), "opencode");
        let meta = fm.metadata.unwrap();
        assert_eq!(meta["audience"], "maintainers");
        assert!(body.contains("Draft notes"));
    }

    #[test]
    fn test_parse_no_frontmatter_returns_none() {
        assert!(parse_skill_md("Just plain text").is_none());
    }

    #[test]
    fn test_parse_missing_closing_delimiter() {
        assert!(parse_skill_md("---\nname: x\ndescription: y\n").is_none());
    }

    #[test]
    fn test_parse_empty_name_returns_none() {
        let raw = "---\nname: \"\"\ndescription: Something\n---\nBody";
        assert!(parse_skill_md(raw).is_none());
    }

    #[test]
    fn test_parse_missing_description_returns_none() {
        let raw = "---\nname: weather\n---\nBody";
        assert!(parse_skill_md(raw).is_none());
    }

    #[test]
    fn test_parse_description_too_long_returns_none() {
        let desc = "a".repeat(1025);
        let raw = format!("---\nname: weather\ndescription: {}\n---\nBody", desc);
        assert!(parse_skill_md(&raw).is_none());
    }

    #[test]
    fn test_parse_empty_body() {
        let raw = "---\nname: weather\ndescription: Weather\n---\n";
        let (_, body) = parse_skill_md(raw).unwrap();
        assert!(body.is_empty());
    }

    #[test]
    fn test_body_excludes_frontmatter() {
        let raw = "---\nname: test\ndescription: Test skill\n---\nHello world";
        let (_, body) = parse_skill_md(raw).unwrap();
        assert!(!body.contains("name:"));
        assert!(body.contains("Hello world"));
    }
}
