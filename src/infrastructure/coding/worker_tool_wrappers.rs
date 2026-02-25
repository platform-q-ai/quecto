//! Tool trait wrappers around worker_tools pure functions.
//!
//! Each wrapper holds a `PathBuf` to the job directory, parses JSON
//! arguments, delegates to the corresponding pure function, and
//! serializes the result as JSON in `ToolResult.content`.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

use super::worker_tools;

// ── Helpers ─────────────────────────────────────────────────────────────

/// Hard errors are tool-level failures the LLM cannot recover from
/// (path violations, I/O errors). Soft failures (ambiguity, not-found)
/// return `is_error = false` with details in the JSON content.
fn is_hard_error(error: &Option<String>) -> bool {
    match error {
        Some(msg) => {
            msg.contains("path violation")
                || msg.contains("cannot read")
                || msg.contains("write failed")
        }
        None => false,
    }
}

fn parse_args(arguments: &str) -> Result<serde_json::Value, DomainError> {
    serde_json::from_str(arguments).map_err(|e| DomainError::Tool(e.to_string()))
}

fn require_str<'a>(args: &'a serde_json::Value, field: &str) -> Result<&'a str, DomainError> {
    args[field]
        .as_str()
        .ok_or_else(|| DomainError::Tool(format!("missing '{field}' argument")))
}

// ── WorkerEditTool ──────────────────────────────────────────────────────

pub struct WorkerEditTool {
    job_dir: PathBuf,
}

impl WorkerEditTool {
    pub fn new(job_dir: PathBuf) -> Self {
        Self { job_dir }
    }
}

impl Tool for WorkerEditTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "worker_edit".to_string(),
            description: "Edit a file by exact string replacement within \
                          the job directory. Returns a unified diff on success."
                .to_string(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Relative path to the file within the job directory"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "Exact string to search for"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "Replacement string"
                    },
                    "preview_only": {
                        "type": "boolean",
                        "description": "If true, compute diff but do not write"
                    },
                    "fuzzy": {
                        "type": "boolean",
                        "description": "If true, try whitespace-trimmed matching on miss"
                    }
                },
                "required": ["file_path", "old_string", "new_string"]
            })
            .to_string(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args_str = arguments.to_string();
        Box::pin(async move {
            let args = parse_args(&args_str)?;
            let file_path = require_str(&args, "file_path")?;
            let old_string = require_str(&args, "old_string")?;
            let new_string = require_str(&args, "new_string")?;
            let preview_only = args["preview_only"].as_bool().unwrap_or(false);
            let fuzzy = args["fuzzy"].as_bool().unwrap_or(false);

            let result = worker_tools::edit_file(&worker_tools::EditParams {
                job_dir: &self.job_dir,
                file_path,
                old_string,
                new_string,
                preview_only,
                fuzzy,
            });

            let is_error = is_hard_error(&result.error);
            let json = serde_json::json!({
                "ok": result.ok,
                "diff": result.diff,
                "first_changed_line": result.first_changed_line,
                "error": result.error,
                "match_count": result.match_count,
                "match_lines": result.match_lines,
                "fuzzy_used": result.fuzzy_used,
            });

            Ok(ToolResult {
                content: json.to_string(),
                is_error,
            })
        })
    }
}

// ── WorkerGrepTool ──────────────────────────────────────────────────────

pub struct WorkerGrepTool {
    job_dir: PathBuf,
}

impl WorkerGrepTool {
    pub fn new(job_dir: PathBuf) -> Self {
        Self { job_dir }
    }
}

impl Tool for WorkerGrepTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "worker_grep".to_string(),
            description: "Search for a pattern in all files under the job \
                          directory, respecting .gitignore by default."
                .to_string(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Substring pattern to search for"
                    },
                    "gitignore": {
                        "type": "boolean",
                        "description": "Whether to respect .gitignore (default true)"
                    }
                },
                "required": ["pattern"]
            })
            .to_string(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args_str = arguments.to_string();
        Box::pin(async move {
            let args = parse_args(&args_str)?;
            let pattern = require_str(&args, "pattern")?;
            let gitignore = args["gitignore"].as_bool().unwrap_or(true);

            let result = worker_tools::grep_content(&self.job_dir, pattern, gitignore);

            let matches: Vec<serde_json::Value> = result
                .matches
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "file": m.file,
                        "line": m.line,
                        "text": m.text,
                    })
                })
                .collect();

            let json = serde_json::json!({
                "ok": result.ok,
                "matches": matches,
                "error": result.error,
            });

            Ok(ToolResult {
                content: json.to_string(),
                is_error: false,
            })
        })
    }
}

// ── WorkerFindTool ──────────────────────────────────────────────────────

pub struct WorkerFindTool {
    job_dir: PathBuf,
}

impl WorkerFindTool {
    pub fn new(job_dir: PathBuf) -> Self {
        Self { job_dir }
    }
}

impl Tool for WorkerFindTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "worker_find".to_string(),
            description: "Find files matching a glob pattern under the job \
                          directory, respecting .gitignore by default."
                .to_string(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "glob": {
                        "type": "string",
                        "description": "Glob pattern to match (e.g. **/*.rs)"
                    },
                    "gitignore": {
                        "type": "boolean",
                        "description": "Whether to respect .gitignore (default true)"
                    }
                },
                "required": ["glob"]
            })
            .to_string(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args_str = arguments.to_string();
        Box::pin(async move {
            let args = parse_args(&args_str)?;
            let glob = require_str(&args, "glob")?;
            let gitignore = args["gitignore"].as_bool().unwrap_or(true);

            let result = worker_tools::find_files(&self.job_dir, glob, gitignore);

            let json = serde_json::json!({
                "ok": result.ok,
                "files": result.files,
                "error": result.error,
            });

            Ok(ToolResult {
                content: json.to_string(),
                is_error: false,
            })
        })
    }
}

// ── WorkerReadTool ──────────────────────────────────────────────────────

pub struct WorkerReadTool {
    job_dir: PathBuf,
}

impl WorkerReadTool {
    pub fn new(job_dir: PathBuf) -> Self {
        Self { job_dir }
    }
}

impl Tool for WorkerReadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "worker_read".to_string(),
            description: "Read a file with pagination support. Returns the \
                          content of the specified line range."
                .to_string(),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Relative path to the file within the job directory"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Line offset to start reading from (0-indexed, default 0)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to return (default 200)"
                    }
                },
                "required": ["file_path"]
            })
            .to_string(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args_str = arguments.to_string();
        Box::pin(async move {
            let args = parse_args(&args_str)?;
            let file_path = require_str(&args, "file_path")?;
            let offset = args["offset"].as_u64().unwrap_or(0) as usize;
            let limit = args["limit"].as_u64().unwrap_or(200) as usize;

            let result = worker_tools::read_file_paginated(&self.job_dir, file_path, offset, limit);

            let json = serde_json::json!({
                "ok": result.ok,
                "content": result.content,
                "total_lines": result.total_lines,
                "offset": result.offset,
                "limit": result.limit,
                "has_more": result.has_more,
                "error": result.error,
            });

            Ok(ToolResult {
                content: json.to_string(),
                is_error: false,
            })
        })
    }
}

// ── Registry builder ────────────────────────────────────────────────────

/// Build a `ToolRegistryImpl` containing only the worker coding tools.
///
/// This registry is used inside the nsjail worker process. It contains
/// no exec, spawn, or cron tools — only the coding-specific tools.
pub fn build_worker_tool_registry(
    job_dir: PathBuf,
) -> crate::infrastructure::tools::registry::ToolRegistryImpl {
    use std::sync::Arc;
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register(Arc::new(WorkerEditTool::new(job_dir.clone())));
    registry.register(Arc::new(WorkerGrepTool::new(job_dir.clone())));
    registry.register(Arc::new(WorkerFindTool::new(job_dir.clone())));
    registry.register(Arc::new(WorkerReadTool::new(job_dir)));
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_job_dir() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let job_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(job_dir.join("src")).unwrap();
        std::fs::write(
            job_dir.join("src/main.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();
        std::fs::write(
            job_dir.join("src/lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
        )
        .unwrap();
        std::fs::write(job_dir.join("README.md"), "# My App\n\nA sample project.\n").unwrap();
        std::fs::write(job_dir.join(".gitignore"), "target/\n*.log\n").unwrap();
        (tmp, job_dir)
    }

    #[test]
    fn test_worker_edit_definition() {
        let (_tmp, job_dir) = setup_job_dir();
        let tool = WorkerEditTool::new(job_dir);
        let def = tool.definition();
        assert_eq!(def.name, "worker_edit");
        let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("file_path")));
        assert!(required.contains(&serde_json::json!("old_string")));
        assert!(required.contains(&serde_json::json!("new_string")));
    }

    #[tokio::test]
    async fn test_worker_edit_success() {
        let (_tmp, job_dir) = setup_job_dir();
        let tool = WorkerEditTool::new(job_dir);
        let result = tool
            .execute(r#"{"file_path":"src/main.rs","old_string":"hello","new_string":"world"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        let json: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(json["ok"], true);
        assert!(json["diff"].as_str().unwrap().contains("+"));
    }

    #[tokio::test]
    async fn test_worker_edit_missing_arg() {
        let (_tmp, job_dir) = setup_job_dir();
        let tool = WorkerEditTool::new(job_dir);
        let result = tool.execute(r#"{"file_path":"src/main.rs"}"#).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("old_string"));
    }

    #[test]
    fn test_worker_grep_definition() {
        let (_tmp, job_dir) = setup_job_dir();
        let tool = WorkerGrepTool::new(job_dir);
        let def = tool.definition();
        assert_eq!(def.name, "worker_grep");
    }

    #[tokio::test]
    async fn test_worker_grep_finds_matches() {
        let (_tmp, job_dir) = setup_job_dir();
        let tool = WorkerGrepTool::new(job_dir);
        let result = tool.execute(r#"{"pattern":"println"}"#).await.unwrap();
        let json: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(json["ok"], true);
        assert!(!json["matches"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_worker_find_definition() {
        let (_tmp, job_dir) = setup_job_dir();
        let tool = WorkerFindTool::new(job_dir);
        let def = tool.definition();
        assert_eq!(def.name, "worker_find");
    }

    #[tokio::test]
    async fn test_worker_find_matches() {
        let (_tmp, job_dir) = setup_job_dir();
        let tool = WorkerFindTool::new(job_dir);
        let result = tool.execute(r#"{"glob":"**/*.rs"}"#).await.unwrap();
        let json: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(json["ok"], true);
        let files = json["files"].as_array().unwrap();
        assert!(files.iter().any(|f| f.as_str() == Some("src/main.rs")));
    }

    #[test]
    fn test_worker_read_definition() {
        let (_tmp, job_dir) = setup_job_dir();
        let tool = WorkerReadTool::new(job_dir);
        let def = tool.definition();
        assert_eq!(def.name, "worker_read");
    }

    #[tokio::test]
    async fn test_worker_read_success() {
        let (_tmp, job_dir) = setup_job_dir();
        let tool = WorkerReadTool::new(job_dir);
        let result = tool
            .execute(r#"{"file_path":"src/main.rs"}"#)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(json["ok"], true);
        assert!(json["content"].as_str().unwrap().contains("fn main()"));
    }

    #[test]
    fn test_build_worker_tool_registry() {
        let (_tmp, job_dir) = setup_job_dir();
        let registry = build_worker_tool_registry(job_dir);
        let names = registry.names();
        assert_eq!(names.len(), 4);
        assert!(registry.get("worker_edit").is_some());
        assert!(registry.get("worker_grep").is_some());
        assert!(registry.get("worker_find").is_some());
        assert!(registry.get("worker_read").is_some());
    }

    #[tokio::test]
    async fn test_worker_edit_path_violation() {
        let (_tmp, job_dir) = setup_job_dir();
        let tool = WorkerEditTool::new(job_dir);
        let result = tool
            .execute(r#"{"file_path":"../etc/passwd","old_string":"root","new_string":"hack"}"#)
            .await
            .unwrap();
        assert!(result.is_error, "path violation should be is_error=true");
        let json: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(json["ok"], false);
        assert!(json["error"].as_str().unwrap().contains("path violation"));
    }

    #[tokio::test]
    async fn test_worker_read_path_violation() {
        let (_tmp, job_dir) = setup_job_dir();
        let tool = WorkerReadTool::new(job_dir);
        let result = tool
            .execute(r#"{"file_path":"../../etc/shadow"}"#)
            .await
            .unwrap();
        // read_file_paginated returns ok=false with path violation in error field
        // The wrapper does NOT set is_error for read results (only edit uses is_hard_error)
        let json: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(json["ok"], false);
        assert!(json["error"].as_str().unwrap().contains("path violation"));
    }

    #[tokio::test]
    async fn test_registry_execute_unknown_tool() {
        let (_tmp, job_dir) = setup_job_dir();
        let registry = build_worker_tool_registry(job_dir);
        let result = registry.execute("nonexistent", "{}").await;
        assert!(result.is_err());
    }
}
