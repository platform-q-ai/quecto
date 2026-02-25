//! Artifact export for coding jobs.
//!
//! After a worker completes (success or failure), the coordinator captures
//! artifacts from the job directory: git patches, commit metadata, test
//! output, run logs, skills applied, spawn logs, and a structured summary.
//! Artifacts are stored in the per-job artifact directory.

use std::path::{Path, PathBuf};

use crate::domain::coding_event::{EventEnvelope, EventSource};

// ── Configuration ───────────────────────────────────────────────────────

/// Limits and settings for artifact export.
#[derive(Debug, Clone)]
pub struct ExportConfig {
    /// Maximum run log size in bytes before truncation.
    pub max_log_bytes: usize,
    /// Truncation marker appended when a log is cut.
    pub truncation_marker: String,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            max_log_bytes: 1_048_576, // 1 MiB
            truncation_marker: "\n--- [truncated] ---\n".to_string(),
        }
    }
}

// ── Result types ────────────────────────────────────────────────────────

/// Outcome of a single artifact export step.
#[derive(Debug, Clone)]
pub struct ArtifactResult {
    /// Name of the artifact file (e.g. "patch.diff").
    pub name: String,
    /// Absolute path where it was written.
    pub path: PathBuf,
    /// Size in bytes.
    pub size_bytes: u64,
}

/// Outcome of a full export run.
#[derive(Debug, Clone)]
pub struct ExportResult {
    /// Artifacts that were actually written.
    pub artifacts: Vec<ArtifactResult>,
    /// Events emitted during export.
    pub events: Vec<EventEnvelope>,
}

// ── Parameter structs (to stay within 4-arg limit) ──────────────────────

/// Parameters for the full export pipeline.
#[derive(Debug, Clone)]
pub struct ExportParams {
    /// Job identifier (e.g. "job_000001").
    pub job_id: String,
    /// Run identifier.
    pub run_id: String,
    /// Path to the job's repo directory (contains .git).
    pub job_repo_dir: PathBuf,
    /// Path to the job's artifact directory (will be created if absent).
    pub artifacts_dir: PathBuf,
    /// Export configuration.
    pub config: ExportConfig,
}

/// Parameters for the summary artifact.
#[derive(Debug, Clone)]
pub struct SummaryParams {
    pub goal: String,
    pub state: String,
    pub summary_text: Option<String>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
}

/// Parameters for skills snapshot.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub source: String,
}

/// Parameters for a spawn log entry.
#[derive(Debug, Clone)]
pub struct SpawnLogEntry {
    pub agent_id: String,
    pub request: String,
    pub decision: String,
    pub result: String,
}

// ── ArtifactExporter ────────────────────────────────────────────────────

/// Captures and writes job artifacts to the artifacts directory.
#[derive(Debug)]
pub struct ArtifactExporter {
    params: ExportParams,
    seq: u64,
    artifacts: Vec<ArtifactResult>,
    events: Vec<EventEnvelope>,
}

impl ArtifactExporter {
    /// Create a new exporter. Creates the artifacts directory if needed.
    pub fn new(params: ExportParams) -> std::io::Result<Self> {
        std::fs::create_dir_all(&params.artifacts_dir)?;
        Ok(Self {
            params,
            seq: 0,
            artifacts: Vec::new(),
            events: Vec::new(),
        })
    }

    /// Export a git diff patch from the job repo.
    pub fn export_patch(&mut self) -> std::io::Result<()> {
        let diff = run_git_diff(&self.params.job_repo_dir);
        let redacted = redact_api_keys(&diff);
        if redacted.trim().is_empty() {
            return Ok(());
        }
        let path = self.params.artifacts_dir.join("patch.diff");
        std::fs::write(&path, &redacted)?;
        let size = redacted.len() as u64;
        self.record_artifact(&RecordInfo {
            name: "patch.diff",
            path: &path,
            size,
            artifact_type: "patch",
        });
        Ok(())
    }

    /// Export commit metadata as JSON.
    pub fn export_commits(&mut self) -> std::io::Result<()> {
        let commits = collect_commits(&self.params.job_repo_dir);
        if commits.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&commits).unwrap_or_else(|_| "[]".to_string());
        let redacted = redact_api_keys(&json);
        let path = self.params.artifacts_dir.join("commits.json");
        std::fs::write(&path, &redacted)?;
        let size = redacted.len() as u64;
        self.record_artifact(&RecordInfo {
            name: "commits.json",
            path: &path,
            size,
            artifact_type: "commits",
        });
        Ok(())
    }

    /// Export a run log, truncating if it exceeds the configured limit.
    pub fn export_run_log(&mut self, log_content: &str) -> std::io::Result<()> {
        let redacted = redact_api_keys(log_content);
        let max = self.params.config.max_log_bytes;
        let marker = &self.params.config.truncation_marker;
        let content = truncate_log(&redacted, max, marker);
        let path = self.params.artifacts_dir.join("run.log");
        std::fs::write(&path, &content)?;
        let size = content.len() as u64;
        self.record_artifact(&RecordInfo {
            name: "run.log",
            path: &path,
            size,
            artifact_type: "log",
        });
        Ok(())
    }

    /// Export a structured summary.
    pub fn export_summary(&mut self, summary: &SummaryParams) -> std::io::Result<()> {
        let artifact_names: Vec<&str> = self.artifacts.iter().map(|a| a.name.as_str()).collect();
        let obj = build_summary_json(summary, &artifact_names);
        let json = serde_json::to_string_pretty(&obj).unwrap_or_else(|_| "{}".to_string());
        let redacted = redact_api_keys(&json);
        let path = self.params.artifacts_dir.join("summary.json");
        std::fs::write(&path, &redacted)?;
        let size = redacted.len() as u64;
        self.record_artifact(&RecordInfo {
            name: "summary.json",
            path: &path,
            size,
            artifact_type: "summary",
        });
        Ok(())
    }

    /// Export test output.
    pub fn export_test_output(&mut self, content: &str) -> std::io::Result<()> {
        if content.trim().is_empty() {
            return Ok(());
        }
        let redacted = redact_api_keys(content);
        let path = self.params.artifacts_dir.join("test_output.log");
        std::fs::write(&path, &redacted)?;
        let size = redacted.len() as u64;
        self.record_artifact(&RecordInfo {
            name: "test_output.log",
            path: &path,
            size,
            artifact_type: "test_output",
        });
        Ok(())
    }

    /// Export skills applied snapshot.
    pub fn export_skills(&mut self, skills: &[SkillEntry]) -> std::io::Result<()> {
        let arr: Vec<serde_json::Value> = skills
            .iter()
            .map(|s| serde_json::json!({"name": s.name, "source": s.source}))
            .collect();
        let json = serde_json::to_string_pretty(&arr).unwrap_or_else(|_| "[]".to_string());
        let path = self.params.artifacts_dir.join("skills_applied.json");
        std::fs::write(&path, &json)?;
        let size = json.len() as u64;
        self.record_artifact(&RecordInfo {
            name: "skills_applied.json",
            path: &path,
            size,
            artifact_type: "skills",
        });
        Ok(())
    }

    /// Export spawn log.
    pub fn export_spawn_log(&mut self, entries: &[SpawnLogEntry]) -> std::io::Result<()> {
        let arr: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "agent_id": e.agent_id,
                    "request": e.request,
                    "decision": e.decision,
                    "result": e.result,
                })
            })
            .collect();
        let json = serde_json::to_string_pretty(&arr).unwrap_or_else(|_| "[]".to_string());
        let path = self.params.artifacts_dir.join("spawn_log.json");
        std::fs::write(&path, &json)?;
        let size = json.len() as u64;
        self.record_artifact(&RecordInfo {
            name: "spawn_log.json",
            path: &path,
            size,
            artifact_type: "spawn_log",
        });
        Ok(())
    }

    /// Finalize and return the export result.
    pub fn finish(self) -> ExportResult {
        ExportResult {
            artifacts: self.artifacts,
            events: self.events,
        }
    }

    /// List artifact file names that were written.
    pub fn artifact_names(&self) -> Vec<String> {
        self.artifacts.iter().map(|a| a.name.clone()).collect()
    }

    // ── internal ────────────────────────────────────────────────────────

    fn record_artifact(&mut self, info: &RecordInfo<'_>) {
        self.artifacts.push(ArtifactResult {
            name: info.name.to_string(),
            path: info.path.to_path_buf(),
            size_bytes: info.size,
        });
        self.seq += 1;
        let rel_path = relative_to_job(info.path, &self.params.artifacts_dir, info.name);
        let event = make_artifact_event(&ArtifactEventParams {
            run_id: &self.params.run_id,
            job_id: &self.params.job_id,
            seq: self.seq,
            artifact_type: info.artifact_type,
            path: &rel_path,
            size_bytes: info.size,
        });
        self.events.push(event);
    }
}

/// Internal bookkeeping for a single artifact.
struct RecordInfo<'a> {
    name: &'a str,
    path: &'a Path,
    size: u64,
    artifact_type: &'a str,
}

// ── Git helpers ─────────────────────────────────────────────────────────

fn run_git_diff(repo_dir: &Path) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["diff", "HEAD~1..HEAD"])
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => {
            // Fallback: diff against empty tree for initial commits
            let output2 = std::process::Command::new("git")
                .arg("-C")
                .arg(repo_dir)
                .args(["diff", "--cached"])
                .output();
            match output2 {
                Ok(o2) => String::from_utf8_lossy(&o2.stdout).to_string(),
                Err(_) => String::new(),
            }
        }
    }
}

/// Represents a single commit entry in the commits.json artifact.
#[derive(Debug, serde::Serialize)]
struct CommitEntry {
    hash: String,
    message: String,
    author: String,
    timestamp: String,
}

fn collect_commits(repo_dir: &Path) -> Vec<CommitEntry> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["log", "--format=%H%n%s%n%an%n%ai", "--reverse"])
        .output();
    let stdout = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return Vec::new(),
    };
    parse_commit_log(&stdout)
}

fn parse_commit_log(stdout: &str) -> Vec<CommitEntry> {
    let lines: Vec<&str> = stdout.lines().collect();
    let mut entries = Vec::new();
    let mut i = 0;
    while i + 3 < lines.len() {
        entries.push(CommitEntry {
            hash: lines[i].to_string(),
            message: lines[i + 1].to_string(),
            author: lines[i + 2].to_string(),
            timestamp: lines[i + 3].to_string(),
        });
        i += 4;
    }
    entries
}

// ── Log truncation ──────────────────────────────────────────────────────

fn truncate_log(content: &str, max_bytes: usize, marker: &str) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    let cut = safe_utf8_cut(content, max_bytes.saturating_sub(marker.len()));
    let mut result = content[..cut].to_string();
    result.push_str(marker);
    result
}

/// Find a safe UTF-8 cut point at or before `max_bytes`.
fn safe_utf8_cut(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    let mut cut = max_bytes;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

// ── API key redaction ───────────────────────────────────────────────────

/// Redact API keys from artifact content.
///
/// Matches the same patterns as `infrastructure::logging::redact_api_keys`:
/// - `sk-...` (OpenAI / Anthropic)
/// - `gsk_...` / `gsk-...` (Groq)
pub fn redact_api_keys(input: &str) -> String {
    if !input.contains("sk-") && !input.contains("gsk_") && !input.contains("gsk-") {
        return input.to_string();
    }
    let mut result = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if let Some((len, prefix)) = detect_key(&input[i..]) {
            result.push_str(prefix);
            result.push_str("***");
            i += len;
        } else {
            let ch = input[i..].chars().next().unwrap_or_default();
            result.push(ch);
            i += ch.len_utf8();
        }
    }
    result
}

fn detect_key(s: &str) -> Option<(usize, &'static str)> {
    if s.starts_with("sk-ant-") {
        key_len(s, 8).map(|l| (l, "sk-ant-"))
    } else if s.starts_with("sk-") {
        key_len(s, 8).map(|l| (l, "sk-"))
    } else if s.starts_with("gsk_") {
        key_len(s, 12).map(|l| (l, "gsk_"))
    } else if s.starts_with("gsk-") {
        key_len(s, 12).map(|l| (l, "gsk-"))
    } else {
        None
    }
}

fn key_len(s: &str, min: usize) -> Option<usize> {
    let len = s
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .count();
    if len >= min { Some(len) } else { None }
}

// ── Summary builder ─────────────────────────────────────────────────────

fn build_summary_json(summary: &SummaryParams, artifact_names: &[&str]) -> serde_json::Value {
    let mut obj = serde_json::json!({
        "goal": summary.goal,
        "state": summary.state,
        "artifacts": artifact_names,
    });
    if let Some(ref text) = summary.summary_text {
        obj["summary"] = serde_json::json!(text);
    }
    if let Some(ref code) = summary.error_code {
        obj["error_code"] = serde_json::json!(code);
    }
    if let Some(ref detail) = summary.error_detail {
        obj["error_detail"] = serde_json::json!(detail);
    }
    obj
}

// ── Event builder ───────────────────────────────────────────────────────

struct ArtifactEventParams<'a> {
    run_id: &'a str,
    job_id: &'a str,
    seq: u64,
    artifact_type: &'a str,
    path: &'a str,
    size_bytes: u64,
}

fn make_artifact_event(p: &ArtifactEventParams<'_>) -> EventEnvelope {
    EventEnvelope {
        v: "1.0".to_string(),
        ts: chrono::Utc::now().to_rfc3339(),
        run_id: p.run_id.to_string(),
        job_id: p.job_id.to_string(),
        source: EventSource::Coordinator,
        event_type: "artifact.created".to_string(),
        seq: p.seq,
        payload: serde_json::json!({
            "artifact_id": format!("{}:{}", p.job_id, p.artifact_type),
            "artifact_type": p.artifact_type,
            "path": p.path,
            "size_bytes": p.size_bytes,
        }),
    }
}

fn relative_to_job(path: &Path, artifacts_dir: &Path, fallback: &str) -> String {
    // Walk up from artifacts_dir to find the job directory (parent of artifacts/)
    let job_dir = artifacts_dir.parent().unwrap_or(artifacts_dir);
    path.strip_prefix(job_dir)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| format!("artifacts/{fallback}"))
}

// ── Unit tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_log_under_limit() {
        let content = "short log";
        let result = truncate_log(content, 100, "\n--- [truncated] ---\n");
        assert_eq!(result, content);
    }

    #[test]
    fn test_truncate_log_over_limit() {
        let content = "a".repeat(200);
        let result = truncate_log(&content, 100, "[CUT]");
        assert!(result.len() <= 100);
        assert!(result.ends_with("[CUT]"));
    }

    #[test]
    fn test_safe_utf8_cut() {
        let s = "hello world";
        assert_eq!(safe_utf8_cut(s, 5), 5);
        assert_eq!(safe_utf8_cut(s, 100), s.len());
    }

    #[test]
    fn test_safe_utf8_cut_multibyte() {
        let s = "héllo";
        let cut = safe_utf8_cut(s, 2);
        assert!(s.is_char_boundary(cut));
    }

    #[test]
    fn test_redact_openai_key() {
        let input = "key=sk-secret-key-12345 done";
        let result = redact_api_keys(input);
        assert!(!result.contains("sk-secret-key-12345"));
        assert!(result.contains("sk-***"));
    }

    #[test]
    fn test_redact_anthropic_key() {
        let input = "sk-ant-abcdefghijk123456789";
        let result = redact_api_keys(input);
        assert!(result.contains("sk-ant-***"));
    }

    #[test]
    fn test_redact_groq_key() {
        let input = "gsk_abcdefghijklmnop";
        let result = redact_api_keys(input);
        assert!(result.contains("gsk_***"));
    }

    #[test]
    fn test_no_redact_clean() {
        let input = "no keys here";
        assert_eq!(redact_api_keys(input), input);
    }

    #[test]
    fn test_parse_commit_log_empty() {
        assert!(parse_commit_log("").is_empty());
    }

    #[test]
    fn test_parse_commit_log_one() {
        let log = "abc123\nfix: stuff\nAlice\n2026-01-01 00:00:00 +0000";
        let entries = parse_commit_log(log);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hash, "abc123");
        assert_eq!(entries[0].message, "fix: stuff");
    }

    #[test]
    fn test_build_summary_success() {
        let s = SummaryParams {
            goal: "fix bug".to_string(),
            state: "succeeded".to_string(),
            summary_text: Some("all pass".to_string()),
            error_code: None,
            error_detail: None,
        };
        let obj = build_summary_json(&s, &["patch.diff"]);
        assert_eq!(obj["state"], "succeeded");
        assert_eq!(obj["summary"], "all pass");
    }

    #[test]
    fn test_build_summary_failure() {
        let s = SummaryParams {
            goal: "fix bug".to_string(),
            state: "failed".to_string(),
            summary_text: None,
            error_code: Some("tool_error".to_string()),
            error_detail: Some("edit ambiguity".to_string()),
        };
        let obj = build_summary_json(&s, &[]);
        assert_eq!(obj["state"], "failed");
        assert_eq!(obj["error_code"], "tool_error");
    }

    #[test]
    fn test_relative_to_job() {
        let artifacts = PathBuf::from("/tmp/jobs/j1/artifacts");
        let file = PathBuf::from("/tmp/jobs/j1/artifacts/patch.diff");
        let rel = relative_to_job(&file, &artifacts, "patch.diff");
        assert_eq!(rel, "artifacts/patch.diff");
    }
}
