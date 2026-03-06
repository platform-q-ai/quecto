//! Script extension: discovers and executes tools defined via `extension.toml`
//! manifests on disk.  Each script tool runs as a subprocess — JSON arguments
//! on stdin, JSON result on stdout.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use crate::domain::error::DomainError;
use crate::domain::extension::Extension;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Parsed `extension.toml` manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct ExtensionManifest {
    pub name: String,
    pub description: String,
    pub parameters_schema: String,
    pub command: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    pub system_prompt: Option<String>,
}

fn default_timeout() -> u64 {
    30
}

impl ExtensionManifest {
    /// Parse a manifest from TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, String> {
        Self::parse_toml_manual(toml_str)
    }

    fn parse_toml_manual(input: &str) -> Result<Self, String> {
        let mut state = TomlParseState::default();

        for line in input.lines() {
            if let Some(ref field) = state.in_multiline {
                if line.contains("\"\"\"") {
                    state.fields.set(field, state.multiline_buf.clone());
                    state.in_multiline = None;
                    state.multiline_buf.clear();
                } else {
                    if !state.multiline_buf.is_empty() {
                        state.multiline_buf.push('\n');
                    }
                    state.multiline_buf.push_str(line);
                }
                continue;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = trimmed.split_once('=') {
                Self::parse_key_value(key.trim(), value.trim(), &mut state)?;
            }
        }

        state.fields.into_manifest()
    }

    fn parse_key_value(key: &str, value: &str, state: &mut TomlParseState) -> Result<(), String> {
        // Check for multi-line string start
        if let Some(rest) = value.strip_prefix("\"\"\"") {
            state.in_multiline = Some(key.to_string());
            if let Some(end) = rest.find("\"\"\"") {
                state.fields.set(key, rest[..end].to_string());
                state.in_multiline = None;
            } else {
                state.multiline_buf = rest.to_string();
            }
            return Ok(());
        }

        // Strip quotes from simple string values
        let unquoted = if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
        {
            &value[1..value.len() - 1]
        } else {
            value
        };

        match key {
            "timeout_secs" => {
                state.fields.timeout_secs = unquoted.parse().map_err(|_| "invalid timeout_secs")?;
            }
            _ => state.fields.set(key, unquoted.to_string()),
        }
        Ok(())
    }
}

/// Mutable state carried through TOML line parsing.
#[derive(Default)]
struct TomlParseState {
    fields: TomlFields,
    in_multiline: Option<String>,
    multiline_buf: String,
}

/// Intermediate struct for collecting parsed TOML fields.
#[derive(Default)]
struct TomlFields {
    name: Option<String>,
    description: Option<String>,
    parameters_schema: Option<String>,
    command: Option<String>,
    timeout_secs: u64,
    system_prompt: Option<String>,
}

impl TomlFields {
    fn set(&mut self, key: &str, value: String) {
        match key {
            "name" => self.name = Some(value),
            "description" => self.description = Some(value),
            "parameters_schema" => self.parameters_schema = Some(value),
            "command" => self.command = Some(value),
            "system_prompt" => self.system_prompt = Some(value),
            _ => {} // ignore unknown keys
        }
    }

    fn into_manifest(self) -> Result<ExtensionManifest, String> {
        Ok(ExtensionManifest {
            name: self.name.ok_or("missing required field: name")?,
            description: self
                .description
                .ok_or("missing required field: description")?,
            parameters_schema: self
                .parameters_schema
                .ok_or("missing required field: parameters_schema")?,
            command: self.command.ok_or("missing required field: command")?,
            timeout_secs: if self.timeout_secs == 0 {
                30
            } else {
                self.timeout_secs
            },
            system_prompt: self.system_prompt,
        })
    }
}

/// A script-based tool that executes an external command.
#[derive(Debug)]
pub struct ScriptTool {
    manifest: ExtensionManifest,
    cwd: PathBuf,
}

impl ScriptTool {
    pub fn new(manifest: ExtensionManifest, cwd: PathBuf) -> Self {
        Self { manifest, cwd }
    }
}

impl ScriptTool {
    /// Resolve command path and run subprocess with timeout.
    async fn run_subprocess(&self, args: &str) -> Result<std::process::Output, DomainError> {
        let timeout = Duration::from_secs(self.manifest.timeout_secs);
        let command_path = self.resolve_command_path();

        let mut child = tokio::process::Command::new(&command_path)
            .current_dir(&self.cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                DomainError::Tool(format!(
                    "failed to execute extension '{}': {}",
                    self.manifest.name, e
                ))
            })?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(args.as_bytes()).await;
            drop(stdin);
        }

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(e)) => Err(DomainError::Tool(format!(
                "extension '{}' execution error: {}",
                self.manifest.name, e
            ))),
            Err(_) => Err(DomainError::Tool(format!(
                "extension '{}' timed out after {}s",
                self.manifest.name, self.manifest.timeout_secs
            ))),
        }
    }

    fn resolve_command_path(&self) -> PathBuf {
        if self.manifest.command.starts_with("./") {
            self.cwd.join(&self.manifest.command)
        } else {
            PathBuf::from(&self.manifest.command)
        }
    }

    /// Parse subprocess output into a `ToolResult`.
    fn parse_output(name: &str, output: std::process::Output) -> Result<ToolResult, DomainError> {
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let content = if stderr.is_empty() {
                format!(
                    "extension '{}' exited with code {}",
                    name,
                    output.status.code().unwrap_or(-1)
                )
            } else {
                stderr
            };
            return Ok(ToolResult {
                content,
                is_error: true,
                image_blocks: vec![],
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        match serde_json::from_str::<serde_json::Value>(&stdout) {
            Ok(val) => {
                let content = val
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let is_error = val
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Ok(ToolResult {
                    content,
                    is_error,
                    image_blocks: vec![],
                })
            }
            Err(_) => Ok(ToolResult {
                content: format!("invalid output from extension '{}': {}", name, stdout),
                is_error: true,
                image_blocks: vec![],
            }),
        }
    }
}

impl Tool for ScriptTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.manifest.name.clone().into(),
            description: self.manifest.description.clone().into(),
            parameters_schema: self.manifest.parameters_schema.clone().into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            let output = match self.run_subprocess(&args).await {
                Ok(o) => o,
                Err(DomainError::Tool(msg)) => {
                    return Ok(ToolResult {
                        content: msg,
                        is_error: true,
                        image_blocks: vec![],
                    });
                }
                Err(e) => return Err(e),
            };
            Self::parse_output(&self.manifest.name, output)
        })
    }
}

/// A script extension wrapping a manifest and its tools.
pub struct ScriptExtension {
    manifest: ExtensionManifest,
    tool: Arc<ScriptTool>,
}

impl std::fmt::Debug for ScriptExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptExtension")
            .field("name", &self.manifest.name)
            .finish()
    }
}

impl Extension for ScriptExtension {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![self.tool.clone() as Arc<dyn Tool>]
    }

    fn system_prompt_snippet(&self) -> Option<String> {
        self.manifest.system_prompt.clone()
    }

    fn is_script(&self) -> bool {
        true
    }
}

/// Scan a directory for `*/extension.toml` and return discovered extensions.
///
/// Directories without `extension.toml` are skipped.  Invalid manifests
/// are logged as warnings and skipped.
pub fn discover_script_extensions(dir: &Path) -> Vec<Arc<dyn Extension>> {
    let mut extensions = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return extensions,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("extension.toml");
        if !manifest_path.exists() {
            continue;
        }

        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("failed to read {:?}: {}", manifest_path, e);
                continue;
            }
        };

        let manifest = match ExtensionManifest::from_toml(&content) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("invalid manifest {:?}: {}", manifest_path, e);
                continue;
            }
        };

        let cwd = path.clone();
        let tool = Arc::new(ScriptTool::new(manifest.clone(), cwd));
        extensions.push(Arc::new(ScriptExtension { manifest, tool }) as Arc<dyn Extension>);
    }

    extensions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_manifest() {
        let toml = r#"
name = "hello"
description = "Say hello"
parameters_schema = '{"type":"object"}'
command = "./hello.sh"
"#;
        let m = ExtensionManifest::from_toml(toml).unwrap();
        assert_eq!(m.name, "hello");
        assert_eq!(m.command, "./hello.sh");
        assert_eq!(m.timeout_secs, 30);
        assert!(m.system_prompt.is_none());
    }

    #[test]
    fn test_parse_with_timeout_and_system_prompt() {
        let toml = r#"
name = "test"
description = "Test tool"
parameters_schema = '{"type":"object"}'
command = "./test.sh"
timeout_secs = 60
system_prompt = "Be helpful."
"#;
        let m = ExtensionManifest::from_toml(toml).unwrap();
        assert_eq!(m.timeout_secs, 60);
        assert_eq!(m.system_prompt.as_deref(), Some("Be helpful."));
    }

    #[test]
    fn test_parse_multiline_description() {
        let toml = r#"
name = "hello"
description = """
Say hello.
Example: {"name": "Alice"}
"""
parameters_schema = '{"type":"object"}'
command = "./hello.sh"
"#;
        let m = ExtensionManifest::from_toml(toml).unwrap();
        assert!(m.description.contains("Say hello."));
        assert!(m.description.contains("Example"));
    }

    #[test]
    fn test_parse_missing_required_field() {
        let toml = r#"
name = "hello"
description = "Say hello"
"#;
        let result = ExtensionManifest::from_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_toml() {
        let result = ExtensionManifest::from_toml("not valid toml {{{{");
        // Should either parse to missing fields or return error
        assert!(result.is_err());
    }

    #[test]
    fn test_discover_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let exts = discover_script_extensions(tmp.path());
        assert!(exts.is_empty());
    }

    #[test]
    fn test_discover_nonexistent_dir() {
        let exts = discover_script_extensions(Path::new("/nonexistent/12345"));
        assert!(exts.is_empty());
    }

    #[test]
    fn test_tool_definition_from_manifest() {
        let manifest = ExtensionManifest {
            name: "mytool".into(),
            description: "does stuff".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
            command: "./tool.sh".into(),
            timeout_secs: 30,
            system_prompt: None,
        };
        let tool = ScriptTool::new(manifest, PathBuf::from("/tmp"));
        let def = tool.definition();
        assert_eq!(def.name.as_ref(), "mytool");
        assert_eq!(def.description.as_ref(), "does stuff");
    }
}
