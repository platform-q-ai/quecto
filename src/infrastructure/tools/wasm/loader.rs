//! WASM tool loader: scans directories for .wasm + .capabilities.json pairs.

use std::path::Path;
use std::sync::Arc;

use super::capabilities::ToolCapabilities;
use super::runtime::WasmToolRuntime;
use super::wrapper::{WasmToolMeta, WasmToolWrapper};

/// Result of loading tools from a directory.
#[derive(Debug)]
pub struct LoadResult {
    /// Successfully loaded tool names.
    pub loaded: Vec<String>,
    /// Tools that failed to load (name, error message).
    pub errors: Vec<(String, String)>,
}

/// Load a capabilities sidecar JSON file for the given tool name.
///
/// Falls back to default capabilities if the file is missing or invalid.
fn load_capabilities(dir: &Path, name: &str) -> ToolCapabilities {
    let caps_path = dir.join(format!("{name}.capabilities.json"));
    if !caps_path.exists() {
        return ToolCapabilities::default();
    }
    match std::fs::read_to_string(&caps_path) {
        Ok(json) => ToolCapabilities::from_json(&json).unwrap_or_else(|e| {
            tracing::warn!("invalid capabilities for '{name}': {e}");
            ToolCapabilities::default()
        }),
        Err(e) => {
            tracing::warn!("failed to read capabilities for '{name}': {e}");
            ToolCapabilities::default()
        }
    }
}

/// Scan a directory for WASM tools and register them with the runtime.
///
/// For each `<name>.wasm` file, looks for a `<name>.capabilities.json`
/// sidecar. If the sidecar is missing, uses default (empty) capabilities.
pub fn load_tools_from_dir(
    dir: &Path,
    runtime: &Arc<WasmToolRuntime>,
) -> Result<LoadResult, String> {
    let mut result = LoadResult {
        loaded: Vec::new(),
        errors: Vec::new(),
    };

    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("failed to read tools dir '{}': {e}", dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "wasm") {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        if stem.is_empty() {
            continue;
        }

        // Read WASM bytes.
        let wasm_bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                let msg = format!("failed to read '{}': {e}", path.display());
                tracing::warn!("{}", msg);
                result.errors.push((stem, msg));
                continue;
            }
        };

        // Try to compile.
        match runtime.prepare(&stem, &wasm_bytes) {
            Ok(_module) => {
                let _caps = load_capabilities(dir, &stem);
                result.loaded.push(stem);
            }
            Err(e) => {
                tracing::warn!("failed to compile WASM module '{stem}': {e}");
                result.errors.push((stem, e));
            }
        }
    }

    result.loaded.sort();
    Ok(result)
}

/// Create a WasmToolWrapper from a registered module.
pub fn create_wrapper(
    runtime: &Arc<WasmToolRuntime>,
    name: &str,
    description: &str,
    schema: &str,
) -> Result<WasmToolWrapper, String> {
    let module = runtime
        .get(name)
        .ok_or_else(|| format!("module '{name}' not found in cache"))?;

    Ok(WasmToolWrapper::new(
        runtime.clone(),
        module,
        WasmToolMeta {
            name: name.to_string(),
            description: description.to_string(),
            schema: schema.to_string(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::tool::Tool;
    use crate::infrastructure::tools::wasm::runtime::WasmRuntimeConfig;
    use tempfile::TempDir;

    fn test_runtime() -> Arc<WasmToolRuntime> {
        Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::default()).unwrap())
    }

    fn minimal_wasm_bytes() -> Vec<u8> {
        wat::parse_str(
            r#"(component
                (core module $m
                    (func (export "memory") (result i32) (i32.const 0))
                    (memory (export "mem") 1)
                )
            )"#,
        )
        .unwrap()
    }

    #[test]
    fn test_load_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let rt = test_runtime();
        let result = load_tools_from_dir(tmp.path(), &rt).unwrap();
        assert!(result.loaded.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_load_valid_wasm() {
        let tmp = TempDir::new().unwrap();
        let rt = test_runtime();

        // Write a valid WASM file.
        let wasm = minimal_wasm_bytes();
        std::fs::write(tmp.path().join("my_tool.wasm"), &wasm).unwrap();

        let result = load_tools_from_dir(tmp.path(), &rt).unwrap();
        assert_eq!(result.loaded, vec!["my_tool"]);
        assert!(result.errors.is_empty());
        assert!(rt.get("my_tool").is_some());
    }

    #[test]
    fn test_load_invalid_wasm() {
        let tmp = TempDir::new().unwrap();
        let rt = test_runtime();

        std::fs::write(tmp.path().join("bad_tool.wasm"), b"not valid wasm").unwrap();

        let result = load_tools_from_dir(tmp.path(), &rt).unwrap();
        assert!(result.loaded.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].0, "bad_tool");
        assert!(rt.get("bad_tool").is_none());
    }

    #[test]
    fn test_load_with_capabilities_sidecar() {
        let tmp = TempDir::new().unwrap();
        let rt = test_runtime();

        let wasm = minimal_wasm_bytes();
        std::fs::write(tmp.path().join("my_tool.wasm"), &wasm).unwrap();
        std::fs::write(
            tmp.path().join("my_tool.capabilities.json"),
            r#"{"cron": true, "workspace": {"read": true}}"#,
        )
        .unwrap();

        let result = load_tools_from_dir(tmp.path(), &rt).unwrap();
        assert_eq!(result.loaded, vec!["my_tool"]);
    }

    #[test]
    fn test_load_nonexistent_dir() {
        let rt = test_runtime();
        let result = load_tools_from_dir(Path::new("/nonexistent/path"), &rt);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_wrapper() {
        let rt = test_runtime();
        let wasm = minimal_wasm_bytes();
        rt.prepare("test", &wasm).unwrap();

        let wrapper = create_wrapper(&rt, "test", "desc", "{}");
        assert!(wrapper.is_ok());
        let w = wrapper.unwrap();
        assert_eq!(w.definition().name, "test");
    }

    #[test]
    fn test_create_wrapper_missing_module() {
        let rt = test_runtime();
        let result = create_wrapper(&rt, "nonexistent", "desc", "{}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }
}
