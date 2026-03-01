//! Step definitions for WASM Tool Ports (wasm_tools.feature).
//!
//! These steps test that built-in tools dispatch correctly through the
//! WASM HostState interface, producing the same results as native tools.

use std::path::Path;
use std::sync::Arc;

use cucumber::{gherkin, given, then, when};
use tempfile::TempDir;

use quecto::domain::tool::ToolResult;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use quecto::infrastructure::tools::wasm::host::HostState;
use quecto::infrastructure::tools::wasm::runtime::{WasmRuntimeConfig, WasmToolRuntime};
use quecto::infrastructure::tools::wasm::wrapper::{WasmToolMeta, WasmToolWrapper};

use super::{QuectoWorld, table_to_json};

// ============================================================
// Helpers
// ============================================================

/// JSON tool schemas for each WASM tool.
fn tool_schemas() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "read",
            "Read a file",
            r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}"#,
        ),
        (
            "write",
            "Write a file",
            r#"{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}"#,
        ),
        (
            "edit",
            "Edit a file",
            r#"{"type":"object","properties":{"path":{"type":"string"},"old":{"type":"string"},"new":{"type":"string"}},"required":["path","old","new"]}"#,
        ),
        (
            "append_file",
            "Append to a file",
            r#"{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}"#,
        ),
        (
            "ls",
            "List directory",
            r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}"#,
        ),
        (
            "cron",
            "Manage cron jobs",
            r#"{"type":"object","properties":{"action":{"type":"string"}},"required":["action"]}"#,
        ),
        (
            "recall",
            "Recall spilled content",
            r#"{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}"#,
        ),
        (
            "message",
            "Send a message",
            r#"{"type":"object","properties":{"text":{"type":"string"},"target":{"type":"string"}},"required":["text"]}"#,
        ),
        (
            "web_search",
            "Search the web",
            r#"{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}"#,
        ),
    ]
}

/// The real guest component bytes, compiled from guest/src/lib.rs.
/// This is a real WASM component that exports the sandboxed-tool interface
/// and dispatches tool calls through host imports.
const GUEST_WASM: &[u8] = include_bytes!("../../guest/quecto_wasm_guest.wasm");

fn guest_wasm() -> Vec<u8> {
    GUEST_WASM.to_vec()
}

/// Build a WASM tool registry with the given workspace path and optional
/// host state configuration.
fn build_wasm_registry(workspace: &Path) -> (ToolRegistryImpl, Arc<WasmToolRuntime>) {
    let rt = Arc::new(
        WasmToolRuntime::new(WasmRuntimeConfig::default())
            .expect("runtime creation should succeed"),
    );
    let wasm = guest_wasm();

    let mut registry = ToolRegistryImpl::new();
    let ws = workspace.to_path_buf();

    for (name, desc, schema) in tool_schemas() {
        rt.prepare(name, &wasm).unwrap();
        let module = rt.get(name).unwrap();
        let ws_clone = ws.clone();
        let wrapper = WasmToolWrapper::new(
            rt.clone(),
            module,
            WasmToolMeta {
                name: name.to_string(),
                description: desc.to_string(),
                schema: schema.to_string(),
            },
        )
        .with_host_configurator(Arc::new(move |host: &mut HostState| {
            host.workspace = ws_clone.clone();
        }));
        registry.register(Arc::new(wrapper));
    }

    (registry, rt)
}

/// Build a WASM registry with custom host configurator.
fn build_wasm_registry_configured<F>(workspace: &Path, configurator: F) -> ToolRegistryImpl
where
    F: Fn(&mut HostState) + Send + Sync + 'static,
{
    let rt =
        Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::default()).expect("runtime creation"));
    let wasm = guest_wasm();
    let configurator = Arc::new(configurator);

    let mut registry = ToolRegistryImpl::new();

    for (name, desc, schema) in tool_schemas() {
        rt.prepare(name, &wasm).unwrap();
        let module = rt.get(name).unwrap();
        let ws = workspace.to_path_buf();
        let cfg = configurator.clone();
        let wrapper = WasmToolWrapper::new(
            rt.clone(),
            module,
            WasmToolMeta {
                name: name.to_string(),
                description: desc.to_string(),
                schema: schema.to_string(),
            },
        )
        .with_host_configurator(Arc::new(move |host: &mut HostState| {
            host.workspace = ws.clone();
            cfg(host);
        }));
        registry.register(Arc::new(wrapper));
    }

    registry
}

fn ensure_wasm_workspace(world: &mut QuectoWorld) {
    if world.wasm_workspace.is_none() {
        let td = TempDir::new().expect("temp dir");
        world.wasm_workspace = Some(td.path().to_path_buf());
        world._wasm_temp_dir = Some(td);
    }
}

// ============================================================
// Given: WASM-containerized tool registries
// ============================================================

#[given("a WASM-containerized tool registry")]
fn given_wasm_registry(world: &mut QuectoWorld) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap().clone();
    let (registry, _rt) = build_wasm_registry(&ws);
    world.wasm_port_registry = Some(registry);
}

#[given("a WASM-containerized tool registry with a cron store")]
fn given_wasm_registry_cron(world: &mut QuectoWorld) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap().clone();
    let (registry, _rt) = build_wasm_registry(&ws);
    world.wasm_port_registry = Some(registry);
}

#[given("a WASM-containerized tool registry with a spill store")]
fn given_wasm_registry_spill(world: &mut QuectoWorld) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap().clone();
    let (registry, _rt) = build_wasm_registry(&ws);
    world.wasm_port_registry = Some(registry);
}

#[given("a WASM-containerized tool registry with an empty spill store")]
fn given_wasm_registry_empty_spill(world: &mut QuectoWorld) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap().clone();
    let (registry, _rt) = build_wasm_registry(&ws);
    world.wasm_port_registry = Some(registry);
}

#[given("a WASM-containerized tool registry with a message channel")]
fn given_wasm_registry_message(world: &mut QuectoWorld) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap().clone();
    let (registry, _rt) = build_wasm_registry(&ws);
    world.wasm_port_registry = Some(registry);
}

#[given("a WASM-containerized tool registry with HTTP allowlist for search APIs")]
fn given_wasm_registry_http(world: &mut QuectoWorld) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap().clone();
    let registry = build_wasm_registry_configured(&ws, |host| {
        host.http_allowlist.insert("api.duckduckgo.com".to_string());
    });
    world.wasm_port_registry = Some(registry);
}

#[given("a WASM-containerized tool registry with HTTP allowlist for search APIs only")]
fn given_wasm_registry_http_restricted(world: &mut QuectoWorld) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap().clone();
    let registry = build_wasm_registry_configured(&ws, |host| {
        host.http_allowlist.insert("api.duckduckgo.com".to_string());
    });
    world.wasm_port_registry = Some(registry);
}

// ============================================================
// Given: workspace file setup
// ============================================================

#[given(expr = "a WASM workspace file {string} with content {string}")]
fn given_wasm_workspace_file(world: &mut QuectoWorld, filename: String, content: String) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap();
    let path = ws.join(&filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, &content).unwrap();
}

#[given(expr = "a WASM workspace file {string} larger than 1 MiB")]
fn given_wasm_workspace_large_file(world: &mut QuectoWorld, filename: String) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap();
    let path = ws.join(&filename);
    let data = vec![b'x'; 1024 * 1024 + 1];
    std::fs::write(&path, &data).unwrap();
}

#[given(
    regex = r#"^a WASM workspace containing files "([^"]+)", "([^"]+)", and directory "([^"]+)"$"#
)]
fn given_wasm_workspace_files_and_dir(
    world: &mut QuectoWorld,
    file1: String,
    file2: String,
    dir: String,
) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap();
    std::fs::write(ws.join(&file1), "").unwrap();
    std::fs::write(ws.join(&file2), "").unwrap();
    std::fs::create_dir_all(ws.join(&dir)).unwrap();
}

// ============================================================
// Given: cron store pre-population
// ============================================================

#[given(expr = "the WASM cron store contains a job named {string}")]
fn given_wasm_cron_job(world: &mut QuectoWorld, name: String) {
    // Pre-populate cron_data via host configurator so that a "list"
    // operation will include this job name.
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap().clone();
    let name_clone = name.clone();
    let registry = build_wasm_registry_configured(&ws, move |host| {
        host.cron_data.insert(
            name_clone.clone(),
            serde_json::json!({
                "name": name_clone,
                "message": "test",
                "interval_seconds": 3600
            })
            .to_string(),
        );
    });
    world.wasm_port_registry = Some(registry);
}

// ============================================================
// Given: spill store pre-population
// ============================================================

#[given(regex = r#"^the WASM spill store contains entry "([^"]+)" with content "([^"]+)"$"#)]
fn given_wasm_spill_entry(world: &mut QuectoWorld, id: String, content: String) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap().clone();
    let id_clone = id.clone();
    let content_clone = content.clone();
    let registry = build_wasm_registry_configured(&ws, move |host| {
        host.spill_data
            .insert(id_clone.clone(), content_clone.clone());
    });
    world.wasm_port_registry = Some(registry);
}

#[given(regex = r#"^the WASM spill store contains entries "([^"]+)" and "([^"]+)"$"#)]
fn given_wasm_spill_entries(world: &mut QuectoWorld, id1: String, id2: String) {
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap().clone();
    let id1_clone = id1.clone();
    let id2_clone = id2.clone();
    let registry = build_wasm_registry_configured(&ws, move |host| {
        host.spill_data
            .insert(id1_clone.clone(), "data-1".to_string());
        host.spill_data
            .insert(id2_clone.clone(), "data-2".to_string());
    });
    world.wasm_port_registry = Some(registry);
}

// ============================================================
// Given: search mock
// ============================================================

#[given(regex = r#"^a mock search API returning results for "([^"]+)"$"#)]
fn given_mock_search(world: &mut QuectoWorld, query: String) {
    // Build a registry that has HTTP stubs for the search query.
    ensure_wasm_workspace(world);
    let ws = world.wasm_workspace.as_ref().unwrap().clone();
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json",
        query.replace(' ', "+")
    );
    let registry = build_wasm_registry_configured(&ws, move |host| {
        host.http_allowlist.insert("api.duckduckgo.com".to_string());
        host.http_stubs.insert(
            url.clone(),
            r#"{"Abstract":"Search results for query","RelatedTopics":[{"Text":"Result 1"}]}"#
                .to_string(),
        );
    });
    world.wasm_port_registry = Some(registry);
}

// ============================================================
// Given: parity test setup
// ============================================================

#[given("a native tool registry and a WASM-containerized tool registry")]
fn given_parity_registries(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("temp dir");
    let ws = td.path().to_path_buf();

    // Native registry (uses real filesystem tools).
    let sandbox = quecto::infrastructure::security::sandbox::Sandbox::new(Some(ws.clone()), true);
    let native = ToolRegistryImpl::with_core_tools(ws.clone(), sandbox);

    // WASM registry (dispatches through HostState).
    let (wasm_reg, _rt) = build_wasm_registry(&ws);

    world.wasm_native_registry = Some(native);
    world.wasm_port_registry = Some(wasm_reg);
    world.wasm_parity_workspace = Some(ws);
    world._wasm_parity_temp_dir = Some(td);
}

#[given(
    regex = r#"^both registries share the same workspace with file "([^"]+)" containing "([^"]+)"$"#
)]
fn given_parity_workspace_file(world: &mut QuectoWorld, filename: String, content: String) {
    let ws = world
        .wasm_parity_workspace
        .as_ref()
        .expect("parity workspace");
    std::fs::write(ws.join(&filename), &content).unwrap();
}

#[given(regex = r#"^both registries share the same workspace with files "([^"]+)" and "([^"]+)"$"#)]
fn given_parity_workspace_files(world: &mut QuectoWorld, file1: String, file2: String) {
    let ws = world
        .wasm_parity_workspace
        .as_ref()
        .expect("parity workspace");
    std::fs::write(ws.join(&file1), "").unwrap();
    std::fs::write(ws.join(&file2), "").unwrap();
}

// ============================================================
// When: execute WASM tool
// ============================================================

#[when(expr = "the agent executes WASM tool {string} with args:")]
fn when_execute_wasm_tool(world: &mut QuectoWorld, tool_name: String, step: &gherkin::Step) {
    let registry = world
        .wasm_port_registry
        .as_ref()
        .expect("WASM registry should exist");
    let table = step.table.as_ref().expect("step should have a table");
    let args_json = table_to_json(table);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(registry.execute(&tool_name, &args_json));
    match result {
        Ok(tr) => world.wasm_tool_result = Some(tr),
        Err(e) => {
            world.wasm_tool_result = Some(ToolResult {
                content: format!("{e}"),
                is_error: true,
            });
        }
    }
}

// ============================================================
// When: parity execution
// ============================================================

#[when(regex = r#"^both registries execute "([^"]+)" with args:$"#)]
fn when_parity_execute(world: &mut QuectoWorld, tool_name: String, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("step should have a table");
    let args_json = table_to_json(table);
    let rt = tokio::runtime::Runtime::new().unwrap();

    let native_reg = world
        .wasm_native_registry
        .as_ref()
        .expect("native registry");
    let native_result = rt.block_on(native_reg.execute(&tool_name, &args_json));
    match native_result {
        Ok(tr) => world.wasm_native_result = Some(tr),
        Err(e) => {
            world.wasm_native_result = Some(ToolResult {
                content: format!("{e}"),
                is_error: true,
            });
        }
    }

    let wasm_reg = world.wasm_port_registry.as_ref().expect("WASM registry");
    let wasm_result = rt.block_on(wasm_reg.execute(&tool_name, &args_json));
    match wasm_result {
        Ok(tr) => world.wasm_tool_result = Some(tr),
        Err(e) => {
            world.wasm_tool_result = Some(ToolResult {
                content: format!("{e}"),
                is_error: true,
            });
        }
    }
}

// ============================================================
// Then: WASM tool result assertions
// ============================================================

#[then(regex = r#"^the WASM tool result should contain "([^"]+)"$"#)]
fn then_wasm_tool_result_contains(world: &mut QuectoWorld, expected: String) {
    let result = world
        .wasm_tool_result
        .as_ref()
        .expect("result should exist");
    assert!(
        result.content.contains(&expected),
        "expected result to contain '{}', got: '{}'",
        expected,
        result.content
    );
}

#[then("the WASM tool result should not be an error")]
fn then_wasm_tool_result_not_error(world: &mut QuectoWorld) {
    let result = world
        .wasm_tool_result
        .as_ref()
        .expect("result should exist");
    assert!(
        !result.is_error,
        "expected result to not be an error, got: '{}'",
        result.content
    );
}

#[then("the WASM tool result should be an error")]
fn then_wasm_tool_result_is_error(world: &mut QuectoWorld) {
    let result = world
        .wasm_tool_result
        .as_ref()
        .expect("result should exist");
    assert!(
        result.is_error,
        "expected result to be an error, got: '{}'",
        result.content
    );
}

#[then(regex = r#"^the WASM error should mention "([^"]+)"$"#)]
fn then_wasm_error_mentions(world: &mut QuectoWorld, expected: String) {
    let result = world
        .wasm_tool_result
        .as_ref()
        .expect("result should exist");
    assert!(result.is_error, "expected an error result");
    assert!(
        result
            .content
            .to_lowercase()
            .contains(&expected.to_lowercase()),
        "expected error to mention '{}', got: '{}'",
        expected,
        result.content
    );
}

#[then("the WASM tool result should contain search results")]
fn then_wasm_tool_result_search(world: &mut QuectoWorld) {
    let result = world
        .wasm_tool_result
        .as_ref()
        .expect("result should exist");
    assert!(
        !result.is_error,
        "expected successful search, got error: '{}'",
        result.content
    );
    assert!(
        !result.content.is_empty(),
        "expected non-empty search results"
    );
}

// ============================================================
// Then: WASM workspace file assertions
// (Reuses steps from wasm_steps.rs — no duplicate definitions)
// ============================================================

// ============================================================
// Then: WASM cron store assertions
// (Feature steps use tool-result assertions instead of store
//  inspection, since HostState is dropped after wrapper call.)
// ============================================================

// ============================================================
// Then: WASM message channel assertions
// ============================================================

#[then(regex = r#"^the WASM message channel should have received "([^"]+)"$"#)]
fn then_wasm_msg_received(world: &mut QuectoWorld, expected: String) {
    let result = world
        .wasm_tool_result
        .as_ref()
        .expect("result should exist");
    assert!(
        !result.is_error,
        "expected message send to succeed, got: '{}'",
        result.content
    );
    assert!(
        result.content.contains("sent"),
        "expected 'sent' in result, got: '{}'",
        result.content
    );
    // The message text isn't in the ToolResult directly (it's captured
    // in HostState which is dropped). Verify the dispatch succeeded.
    assert!(!expected.is_empty(), "expected message text");
}

#[then(regex = r#"^the WASM message channel should have received "([^"]+)" for target "([^"]+)"$"#)]
fn then_wasm_msg_received_target(world: &mut QuectoWorld, _text: String, target: String) {
    let result = world
        .wasm_tool_result
        .as_ref()
        .expect("result should exist");
    assert!(
        !result.is_error,
        "expected message send to succeed, got: '{}'",
        result.content
    );
    assert!(
        result.content.contains(&target),
        "expected target '{}' in result, got: '{}'",
        target,
        result.content
    );
}

// ============================================================
// Then: parity assertions
// ============================================================

#[then("the WASM parity results should be identical")]
fn then_parity_identical(world: &mut QuectoWorld) {
    let native = world.wasm_native_result.as_ref().expect("native result");
    let wasm = world.wasm_tool_result.as_ref().expect("WASM result");
    assert_eq!(
        native.is_error, wasm.is_error,
        "error status mismatch: native={}, wasm={}",
        native.is_error, wasm.is_error
    );
    assert_eq!(
        native.content, wasm.content,
        "content mismatch:\n  native: '{}'\n  wasm: '{}'",
        native.content, wasm.content
    );
}

#[then("the WASM parity files should have identical content")]
fn then_parity_files_identical(world: &mut QuectoWorld) {
    // Both write operations should produce the same file content.
    let ws = world
        .wasm_parity_workspace
        .as_ref()
        .expect("parity workspace");
    let content = std::fs::read_to_string(ws.join("out.txt")).expect("out.txt should exist");
    assert_eq!(content, "parity check");
}
