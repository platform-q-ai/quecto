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

/// The origin of a loaded skill. Currently only workspace is supported;
/// additional sources (global, registry) may be added in the future.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillSource {
    Workspace,
}

/// YAML frontmatter extracted from a SKILL.md file.
///
/// Required fields: `name`, `description`.
/// Optional fields are parsed for forward-compatibility with OpenCode
/// but are not used by quecto's runtime yet.
#[derive(Debug, Clone)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

impl SkillFrontmatter {
    /// Parse simple YAML frontmatter without depending on `serde_yaml`.
    ///
    /// Handles flat `key: value` pairs and a single-level `metadata:` map
    /// with indented `key: value` children. Returns `None` if the required
    /// `name` or `description` fields are missing.
    pub fn parse(yaml: &str) -> Option<Self> {
        let mut name = None;
        let mut description = None;
        let mut license = None;
        let mut compatibility = None;
        let mut metadata: Option<HashMap<String, String>> = None;
        let mut in_metadata = false;

        for line in yaml.lines() {
            // Indented lines belong to the current metadata block
            if in_metadata && (line.starts_with("  ") || line.starts_with('\t')) {
                if let Some((k, v)) = split_kv(line.trim()) {
                    metadata.get_or_insert_with(HashMap::new).insert(k, v);
                }
                continue;
            }
            // Any non-indented line ends the metadata block
            in_metadata = false;

            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = split_kv(trimmed) {
                match key.as_str() {
                    "name" => name = Some(value),
                    "description" => description = Some(value),
                    "license" => license = Some(value),
                    "compatibility" => compatibility = Some(value),
                    "metadata" if value.is_empty() => in_metadata = true,
                    _ => {} // ignore unknown keys
                }
            }
        }

        Some(Self {
            name: name?,
            description: description?,
            license,
            compatibility,
            metadata,
        })
    }
}

/// Split a `key: value` line into `(key, value)`.
/// Strips optional surrounding quotes from the value.
fn split_kv(line: &str) -> Option<(String, String)> {
    let colon = line.find(':')?;
    let key = line[..colon].trim().to_string();
    if key.is_empty() {
        return None;
    }
    let raw_value = line[colon + 1..].trim();
    // Strip surrounding quotes (single or double)
    let value = if (raw_value.starts_with('"') && raw_value.ends_with('"'))
        || (raw_value.starts_with('\'') && raw_value.ends_with('\''))
    {
        raw_value[1..raw_value.len() - 1].to_string()
    } else {
        raw_value.to_string()
    };
    Some((key, value))
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

/// Split a SKILL.md file into its YAML block and body.
///
/// Expects the file to start with `---`, followed by YAML, then a
/// closing `---`. Returns `(yaml_block, body)` where `yaml_block` is
/// the raw YAML text and `body` is everything after the closing
/// delimiter. Returns `None` if delimiters are missing.
///
/// The caller is responsible for deserializing the YAML block.
pub fn split_skill_md(raw: &str) -> Option<(&str, String)> {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_open = &trimmed[3..];
    let close_idx = find_closing_delimiter(after_open)?;
    let yaml_block = &after_open[..close_idx];
    let body_start = close_idx + 3; // skip the closing "---"
    let body = if body_start < after_open.len() {
        after_open[body_start..].trim().to_string()
    } else {
        String::new()
    };
    Some((yaml_block, body))
}

/// Validate that frontmatter fields meet requirements.
pub fn validate_frontmatter(fm: &SkillFrontmatter) -> bool {
    !fm.name.is_empty() && !fm.description.is_empty() && fm.description.len() <= 1024
}

/// Find the position of the closing `---` delimiter.
/// Looks for `\n---` (newline followed by three dashes) to avoid
/// matching `---` inside YAML values.
fn find_closing_delimiter(s: &str) -> Option<usize> {
    for (i, _) in s.match_indices("\n---") {
        let after = i + 4; // skip "\n---"
        if after >= s.len() {
            return Some(i + 1);
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
    fn test_split_basic_frontmatter() {
        let raw = "---\nname: weather\ndescription: Fetch weather\n---\nBody content";
        let (yaml, body) = split_skill_md(raw).unwrap();
        assert!(yaml.contains("name: weather"));
        assert_eq!(body, "Body content");
    }

    #[test]
    fn test_split_no_frontmatter_returns_none() {
        assert!(split_skill_md("Just plain text").is_none());
    }

    #[test]
    fn test_split_missing_closing_delimiter() {
        assert!(split_skill_md("---\nname: x\ndescription: y\n").is_none());
    }

    #[test]
    fn test_split_empty_body() {
        let raw = "---\nname: weather\ndescription: Weather\n---\n";
        let (_, body) = split_skill_md(raw).unwrap();
        assert!(body.is_empty());
    }

    #[test]
    fn test_body_excludes_frontmatter() {
        let raw = "---\nname: test\ndescription: Test skill\n---\nHello world";
        let (_, body) = split_skill_md(raw).unwrap();
        assert!(!body.contains("name:"));
        assert!(body.contains("Hello world"));
    }

    #[test]
    fn test_validate_frontmatter_valid() {
        let fm = SkillFrontmatter {
            name: "test".into(),
            description: "A test skill".into(),
            license: None,
            compatibility: None,
            metadata: None,
        };
        assert!(validate_frontmatter(&fm));
    }

    #[test]
    fn test_validate_frontmatter_empty_name() {
        let fm = SkillFrontmatter {
            name: String::new(),
            description: "Something".into(),
            license: None,
            compatibility: None,
            metadata: None,
        };
        assert!(!validate_frontmatter(&fm));
    }

    #[test]
    fn test_validate_frontmatter_empty_description() {
        let fm = SkillFrontmatter {
            name: "test".into(),
            description: String::new(),
            license: None,
            compatibility: None,
            metadata: None,
        };
        assert!(!validate_frontmatter(&fm));
    }

    #[test]
    fn test_validate_frontmatter_description_too_long() {
        let fm = SkillFrontmatter {
            name: "test".into(),
            description: "a".repeat(1025),
            license: None,
            compatibility: None,
            metadata: None,
        };
        assert!(!validate_frontmatter(&fm));
    }

    // --- SkillFrontmatter::parse tests ---

    #[test]
    fn test_parse_basic_frontmatter() {
        let yaml = "\nname: weather\ndescription: Fetch weather\n";
        let fm = SkillFrontmatter::parse(yaml).unwrap();
        assert_eq!(fm.name, "weather");
        assert_eq!(fm.description, "Fetch weather");
        assert!(fm.license.is_none());
        assert!(fm.compatibility.is_none());
        assert!(fm.metadata.is_none());
    }

    #[test]
    fn test_parse_all_fields() {
        let yaml = "\nname: git-release\ndescription: Create releases\nlicense: MIT\ncompatibility: opencode\nmetadata:\n  audience: maintainers\n  workflow: github\n";
        let fm = SkillFrontmatter::parse(yaml).unwrap();
        assert_eq!(fm.name, "git-release");
        assert_eq!(fm.description, "Create releases");
        assert_eq!(fm.license.as_deref(), Some("MIT"));
        assert_eq!(fm.compatibility.as_deref(), Some("opencode"));
        let meta = fm.metadata.unwrap();
        assert_eq!(meta.get("audience").unwrap(), "maintainers");
        assert_eq!(meta.get("workflow").unwrap(), "github");
    }

    #[test]
    fn test_parse_missing_name_returns_none() {
        let yaml = "\ndescription: Something\n";
        assert!(SkillFrontmatter::parse(yaml).is_none());
    }

    #[test]
    fn test_parse_missing_description_returns_none() {
        let yaml = "\nname: test\n";
        assert!(SkillFrontmatter::parse(yaml).is_none());
    }

    #[test]
    fn test_parse_quoted_values() {
        let yaml = "\nname: \"my-skill\"\ndescription: 'A skill'\n";
        let fm = SkillFrontmatter::parse(yaml).unwrap();
        assert_eq!(fm.name, "my-skill");
        assert_eq!(fm.description, "A skill");
    }

    #[test]
    fn test_parse_ignores_comments() {
        let yaml = "\n# This is a comment\nname: test\ndescription: Test\n";
        let fm = SkillFrontmatter::parse(yaml).unwrap();
        assert_eq!(fm.name, "test");
    }

    #[test]
    fn test_parse_ignores_unknown_keys() {
        let yaml = "\nname: test\ndescription: Test\nunknown_key: whatever\n";
        let fm = SkillFrontmatter::parse(yaml).unwrap();
        assert_eq!(fm.name, "test");
    }
}
