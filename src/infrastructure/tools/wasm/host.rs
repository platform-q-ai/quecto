//! Host-side implementation of the `quecto:tools/host` WIT interface.
//!
//! Each host function mediates access to a real resource (filesystem, HTTP,
//! channels, stores) with validation and enforcement. The WASM module never
//! touches these resources directly.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use wasmtime::component::ResourceTable;
use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView};

/// An HTTP request from a WASM tool.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers_json: String,
    pub body: String,
}

/// State attached to each WASM Store, providing host capabilities.
pub struct HostState {
    /// Workspace root for filesystem operations.
    pub workspace: PathBuf,
    /// Log entries captured during execution.
    pub logs: Vec<LogEntry>,
    /// Maximum log entries before rate-limiting.
    pub max_log_entries: usize,
    /// HTTP allowlist (hostnames permitted for outbound requests).
    pub http_allowlist: HashSet<String>,
    /// Messages sent via send-message (captured for inspection).
    pub sent_messages: Vec<SentMessage>,
    /// Cron store operations performed (captured for inspection).
    pub cron_ops: Vec<StoreOp>,
    /// Spill store operations performed (captured for inspection).
    pub spill_ops: Vec<StoreOp>,
    /// Spill store data (pre-loaded for recall operations).
    pub spill_data: std::collections::HashMap<String, String>,
    /// Cron job data (pre-loaded for list operations).
    pub cron_data: std::collections::HashMap<String, String>,
    /// HTTP response stubs for testing.
    pub http_stubs: std::collections::HashMap<String, String>,
    /// WASI context (required for wasm32-wasip2 components).
    wasi_ctx: WasiCtx,
    /// WASI resource table.
    wasi_table: ResourceTable,
    /// Per-store resource limits used by wasmtime limiter hooks.
    store_limits: StoreLimits,
}

/// A structured log entry from a WASM tool.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: String,
    pub message: String,
}

/// A message sent through the host channel.
#[derive(Debug, Clone)]
pub struct SentMessage {
    pub target: String,
    pub text: String,
}

/// A store operation (cron or spill) performed by a WASM tool.
#[derive(Debug, Clone)]
pub struct StoreOp {
    pub action: String,
    pub payload: String,
}

impl HostState {
    /// Create a new host state for a tool execution.
    pub fn new(workspace: PathBuf, max_log_entries: usize) -> Self {
        Self {
            workspace,
            logs: Vec::new(),
            max_log_entries,
            http_allowlist: HashSet::new(),
            sent_messages: Vec::new(),
            cron_ops: Vec::new(),
            spill_ops: Vec::new(),
            spill_data: std::collections::HashMap::new(),
            cron_data: std::collections::HashMap::new(),
            http_stubs: std::collections::HashMap::new(),
            wasi_ctx: WasiCtxBuilder::new().build(),
            wasi_table: ResourceTable::new(),
            store_limits: StoreLimitsBuilder::new().build(),
        }
    }

    /// Configure the maximum linear-memory size for this invocation.
    pub fn set_memory_limit(&mut self, memory_limit: usize) {
        self.store_limits = StoreLimitsBuilder::new().memory_size(memory_limit).build();
    }

    /// Return mutable store limits for `Store::limiter`.
    pub fn store_limits_mut(&mut self) -> &mut StoreLimits {
        &mut self.store_limits
    }

    /// Validate that a path is within the workspace (public for dispatch).
    pub fn validate_path_public(&self, path: &str) -> Result<PathBuf, String> {
        self.validate_path(path)
    }

    /// Validate that a path is within the workspace.
    fn validate_path(&self, path: &str) -> Result<PathBuf, String> {
        if Path::new(path).is_absolute() {
            return Err(format!("path denied: '{path}' is outside workspace"));
        }

        let workspace_root = std::fs::canonicalize(&self.workspace)
            .map_err(|e| format!("workspace unavailable '{}': {e}", self.workspace.display()))?;
        let mut resolved = workspace_root.clone();

        for component in Path::new(path).components() {
            match component {
                Component::Normal(seg) => {
                    resolved.push(seg);
                    if resolved.exists() {
                        resolved = std::fs::canonicalize(&resolved)
                            .map_err(|e| format!("failed to resolve '{}': {e}", path))?;
                        if !resolved.starts_with(&workspace_root) {
                            return Err(format!("path denied: '{path}' is outside workspace"));
                        }
                    }
                }
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(format!("path denied: '{path}' is outside workspace"));
                }
            }
        }

        Ok(resolved)
    }

    // --- Host function implementations ---

    /// Maximum file size for read operations (1 MiB), matching native ReadFileTool.
    const MAX_READ_SIZE: u64 = 1024 * 1024;

    /// Read a workspace file.
    pub fn workspace_read(&self, path: &str) -> Result<String, String> {
        let full = self.validate_path(path)?;
        // Enforce file size limit before reading (matches native ReadFileTool).
        if let Ok(meta) = std::fs::metadata(&full) {
            if meta.len() > Self::MAX_READ_SIZE {
                return Err(format!(
                    "file too large: {} bytes (max {})",
                    meta.len(),
                    Self::MAX_READ_SIZE
                ));
            }
        }
        std::fs::read_to_string(&full)
            .map_err(|e| format!("failed to read '{}': {e}", full.display()))
    }

    /// Write content to a workspace file.
    pub fn workspace_write(&self, path: &str, content: &str) -> Result<String, String> {
        let full = self.validate_path(path)?;
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("failed to create dirs: {e}"))?;
        }
        std::fs::write(&full, content)
            .map_err(|e| format!("failed to write '{}': {e}", full.display()))?;
        Ok(format!("wrote {} bytes to {path}", content.len()))
    }

    /// Append content to a workspace file.
    pub fn workspace_append(&self, path: &str, content: &str) -> Result<String, String> {
        use std::io::Write;
        let full = self.validate_path(path)?;
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("failed to create dirs: {e}"))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&full)
            .map_err(|e| format!("failed to open '{}': {e}", full.display()))?;
        file.write_all(content.as_bytes())
            .map_err(|e| format!("failed to append: {e}"))?;
        Ok(format!("appended {} bytes to {path}", content.len()))
    }

    /// List a workspace directory.
    pub fn workspace_list_dir(&self, path: &str) -> Result<String, String> {
        let full = self.validate_path(path)?;
        let mut entries: Vec<String> = Vec::new();
        let dir = std::fs::read_dir(&full)
            .map_err(|e| format!("failed to list '{}': {e}", full.display()))?;
        for entry in dir {
            let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let ft = entry
                .file_type()
                .map_err(|e| format!("file type error: {e}"))?;
            if ft.is_dir() {
                entries.push(format!("{name}/"));
            } else {
                entries.push(name);
            }
        }
        entries.sort();
        Ok(entries.join("\n"))
    }

    /// Make an HTTP request (checked against allowlist).
    pub fn http_request(&self, req: &HttpRequest) -> Result<String, String> {
        // Extract host from URL for allowlist check.
        let host = extract_host(&req.url).ok_or_else(|| format!("invalid URL: {}", req.url))?;
        if !self.http_allowlist.contains(&host) {
            return Err(format!(
                "HTTP request denied: host '{host}' not in allowlist"
            ));
        }
        // Check stubs for testing.
        if let Some(response) = self.http_stubs.get(&req.url) {
            return Ok(response.clone());
        }
        // In production, this would delegate to reqwest. For now, return
        // a stub error indicating no real HTTP is available in WASM host tests.
        Err(format!("no HTTP stub configured for URL: {}", req.url))
    }

    /// Send a message through the host channel.
    pub fn send_message(&mut self, target: &str, text: &str) -> Result<String, String> {
        self.sent_messages.push(SentMessage {
            target: target.to_string(),
            text: text.to_string(),
        });
        Ok(format!("message sent to {target}"))
    }

    /// Perform a cron store operation.
    pub fn cron_store_op(&mut self, action: &str, payload: &str) -> Result<String, String> {
        self.cron_ops.push(StoreOp {
            action: action.to_string(),
            payload: payload.to_string(),
        });
        match action {
            "list" => {
                let names: Vec<&str> = self.cron_data.keys().map(|s| s.as_str()).collect();
                Ok(format!("cron jobs: {}", names.join(", ")))
            }
            "add" => {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(payload) {
                    if let Some(name) = parsed.get("name").and_then(|v| v.as_str()) {
                        self.cron_data.insert(name.to_string(), payload.to_string());
                    }
                }
                Ok(format!("cron op '{action}' executed"))
            }
            "remove" => {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(payload) {
                    if let Some(name) = parsed.get("name").and_then(|v| v.as_str()) {
                        self.cron_data.remove(name);
                    }
                }
                Ok(format!("cron op '{action}' executed"))
            }
            _ => Ok(format!("cron op '{action}' executed")),
        }
    }

    /// Perform a spill store operation.
    pub fn spill_store_op(&mut self, action: &str, payload: &str) -> Result<String, String> {
        self.spill_ops.push(StoreOp {
            action: action.to_string(),
            payload: payload.to_string(),
        });
        match action {
            "recall" => {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(payload) {
                    if let Some(id) = parsed.get("id").and_then(|v| v.as_str()) {
                        if let Some(content) = self.spill_data.get(id) {
                            return Ok(content.clone());
                        }
                        return Err(format!("spill entry '{id}' not found"));
                    }
                }
                Ok("spill op 'recall' executed".to_string())
            }
            "list" => {
                let ids: Vec<&str> = self.spill_data.keys().map(|s| s.as_str()).collect();
                Ok(format!("spill entries: {}", ids.join(", ")))
            }
            _ => Ok(format!("spill op '{action}' executed")),
        }
    }

    /// Log a message (rate-limited).
    pub fn log(&mut self, level: &str, message: &str) {
        if self.logs.len() < self.max_log_entries {
            self.logs.push(LogEntry {
                level: level.to_string(),
                message: message.to_string(),
            });
        }
    }
}

// ============================================================
// Implement WasiView so WASI host imports are satisfied.
// ============================================================

impl WasiView for HostState {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.wasi_table
    }

    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi_ctx
    }
}

// ============================================================
// Implement the wasmtime bindgen! generated Host trait.
// This bridges HostState methods to the WIT interface contract.
// ============================================================

impl super::bindings::quecto::tools::host::Host for HostState {
    fn workspace_read(&mut self, path: String) -> Result<String, String> {
        HostState::workspace_read(self, &path)
    }

    fn workspace_write(&mut self, path: String, content: String) -> Result<String, String> {
        HostState::workspace_write(self, &path, &content)
    }

    fn workspace_append(&mut self, path: String, content: String) -> Result<String, String> {
        HostState::workspace_append(self, &path, &content)
    }

    fn workspace_list_dir(&mut self, path: String) -> Result<String, String> {
        HostState::workspace_list_dir(self, &path)
    }

    fn http_request(
        &mut self,
        method: String,
        url: String,
        headers_json: String,
        body: String,
    ) -> Result<String, String> {
        let req = HttpRequest {
            method,
            url,
            headers_json,
            body,
        };
        HostState::http_request(self, &req)
    }

    fn send_message(&mut self, target: String, text: String) -> Result<String, String> {
        HostState::send_message(self, &target, &text)
    }

    fn cron_store_op(&mut self, action: String, payload: String) -> Result<String, String> {
        HostState::cron_store_op(self, &action, &payload)
    }

    fn spill_store_op(&mut self, action: String, payload: String) -> Result<String, String> {
        HostState::spill_store_op(self, &action, &payload)
    }

    fn log(&mut self, level: String, message: String) {
        HostState::log(self, &level, &message);
    }
}

/// Extract the hostname from a URL string.
fn extract_host(url: &str) -> Option<String> {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = without_scheme.split('/').next()?;
    let host = host.split(':').next()?;
    Some(host.to_string())
}

/// Thread-safe wrapper around HostState for use in WASM Store.
pub type SharedHostState = Arc<Mutex<HostState>>;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_host() -> (HostState, TempDir) {
        let tmp = TempDir::new().unwrap();
        let host = HostState::new(tmp.path().to_path_buf(), 100);
        (host, tmp)
    }

    #[test]
    fn test_workspace_read_write() {
        let (host, _tmp) = test_host();
        host.workspace_write("test.txt", "hello").unwrap();
        let content = host.workspace_read("test.txt").unwrap();
        assert_eq!(content, "hello");
    }

    #[test]
    fn test_workspace_read_outside_blocked() {
        let (host, _tmp) = test_host();
        let result = host.workspace_read("/etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("outside workspace"));
    }

    #[test]
    fn test_workspace_path_traversal_blocked() {
        let (host, _tmp) = test_host();
        let result = host.workspace_read("../etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("outside workspace"));
    }

    #[test]
    fn test_workspace_symlink_escape_blocked() {
        let (host, tmp) = test_host();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "top-secret").unwrap();

        std::os::unix::fs::symlink(outside.path(), tmp.path().join("linked")).unwrap();
        let result = host.workspace_read("linked/secret.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("outside workspace"));
    }

    #[test]
    fn test_workspace_list_dir() {
        let (host, _tmp) = test_host();
        host.workspace_write("a.txt", "a").unwrap();
        host.workspace_write("b.txt", "b").unwrap();
        let listing = host.workspace_list_dir(".").unwrap();
        assert!(listing.contains("a.txt"));
        assert!(listing.contains("b.txt"));
    }

    #[test]
    fn test_workspace_append() {
        let (host, _tmp) = test_host();
        host.workspace_write("log.txt", "line1\n").unwrap();
        host.workspace_append("log.txt", "line2\n").unwrap();
        let content = host.workspace_read("log.txt").unwrap();
        assert_eq!(content, "line1\nline2\n");
    }

    #[test]
    fn test_http_request_allowlist_enforced() {
        let (host, _tmp) = test_host();
        let req = HttpRequest {
            method: "GET".into(),
            url: "https://evil.com/data".into(),
            headers_json: String::new(),
            body: String::new(),
        };
        let result = host.http_request(&req);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in allowlist"));
    }

    #[test]
    fn test_http_request_allowed_host() {
        let (mut host, _tmp) = test_host();
        host.http_allowlist.insert("api.example.com".to_string());
        host.http_stubs.insert(
            "https://api.example.com/search".to_string(),
            "results".to_string(),
        );
        let req = HttpRequest {
            method: "GET".into(),
            url: "https://api.example.com/search".into(),
            headers_json: String::new(),
            body: String::new(),
        };
        let result = host.http_request(&req);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "results");
    }

    #[test]
    fn test_send_message() {
        let (mut host, _tmp) = test_host();
        host.send_message("telegram:123", "hello").unwrap();
        assert_eq!(host.sent_messages.len(), 1);
        assert_eq!(host.sent_messages[0].target, "telegram:123");
        assert_eq!(host.sent_messages[0].text, "hello");
    }

    #[test]
    fn test_cron_store_op() {
        let (mut host, _tmp) = test_host();
        host.cron_store_op("add", r#"{"name":"test"}"#).unwrap();
        assert_eq!(host.cron_ops.len(), 1);
        assert_eq!(host.cron_ops[0].action, "add");
    }

    #[test]
    fn test_spill_store_recall() {
        let (mut host, _tmp) = test_host();
        host.spill_data
            .insert("spill-001".to_string(), "big output".to_string());
        let result = host
            .spill_store_op("recall", r#"{"id":"spill-001"}"#)
            .unwrap();
        assert_eq!(result, "big output");
    }

    #[test]
    fn test_spill_store_recall_not_found() {
        let (mut host, _tmp) = test_host();
        let result = host.spill_store_op("recall", r#"{"id":"nonexistent"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_log_rate_limiting() {
        let (mut host, _tmp) = test_host();
        // max_log_entries is 100
        for i in 0..200 {
            host.log("info", &format!("msg {i}"));
        }
        assert_eq!(host.logs.len(), 100);
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(
            extract_host("https://api.example.com/path"),
            Some("api.example.com".to_string())
        );
        assert_eq!(
            extract_host("http://localhost:8080/foo"),
            Some("localhost".to_string())
        );
        assert_eq!(extract_host("not a url"), None);
    }
}
