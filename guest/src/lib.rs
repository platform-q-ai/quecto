//! WASM guest component for quecto sandboxed tools.
//!
//! This crate compiles to a WASM component (`wasm32-wasip2`) that exports the
//! `quecto:tools/tool` interface. Tool logic runs inside the WASM sandbox and
//! accesses host resources exclusively through imported `quecto:tools/host`
//! functions.

// Generate guest-side bindings from the WIT contract.
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../wit/tool.wit",
});

use quecto::tools::host;

/// Guest component implementation.
struct GuestTool;

// Export the tool interface.
export!(GuestTool);

impl exports::quecto::tools::tool::Guest for GuestTool {
    fn execute(params: String) -> Result<String, String> {
        dispatch(&params)
    }

    fn schema() -> String {
        // Schema is provided by the host wrapper (WasmToolMeta).
        // The guest returns a placeholder; the host never calls this
        // in the current architecture.
        String::from(r#"{"type":"object"}"#)
    }

    fn description() -> String {
        // Description is provided by the host wrapper (WasmToolMeta).
        String::from("WASM sandboxed tool")
    }
}

// ============================================================
// Tool dispatch — routes JSON params to host imports.
// ============================================================

/// Dispatch tool execution based on a `__tool` field in the JSON params.
///
/// The host wrapper injects `__tool` into the args before calling execute,
/// so the guest knows which tool logic to run.
fn dispatch(params: &str) -> Result<String, String> {
    let args: serde_json::Value =
        serde_json::from_str(params).map_err(|e| format!("invalid JSON: {e}"))?;

    let tool_name = args
        .get("__tool")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing __tool field".to_string())?;

    match tool_name {
        "read" | "read_file" => dispatch_read_file(&args),
        "write" | "write_file" => dispatch_write_file(&args),
        "edit_file" => dispatch_edit_file(&args),
        "append_file" => dispatch_append_file(&args),
        "list_dir" => dispatch_list_dir(&args),
        "cron" => dispatch_cron(&args),
        "recall" => dispatch_recall(&args),
        "message" => dispatch_message(&args),
        "web_search" => dispatch_web_search(&args),
        other => Err(format!("unknown tool: {other}")),
    }
}

fn get_str<'a>(args: &'a serde_json::Value, field: &str) -> Result<&'a str, String> {
    args.get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing required field: '{field}'"))
}

fn dispatch_read_file(args: &serde_json::Value) -> Result<String, String> {
    let path = get_str(args, "path")?;
    host::workspace_read(path)
}

fn dispatch_write_file(args: &serde_json::Value) -> Result<String, String> {
    let path = get_str(args, "path")?;
    let content = get_str(args, "content")?;
    host::workspace_write(path, content)
}

fn dispatch_edit_file(args: &serde_json::Value) -> Result<String, String> {
    let path = get_str(args, "path")?;
    let old = get_str(args, "old")?;
    let new = get_str(args, "new")?;

    let content = host::workspace_read(path)?;
    if !content.contains(old) {
        return Err(format!("substring not found: '{old}'"));
    }
    let replaced = content.replacen(old, new, 1);
    host::workspace_write(path, &replaced)?;
    Ok(format!("replaced '{old}' with '{new}' in {path}"))
}

fn dispatch_append_file(args: &serde_json::Value) -> Result<String, String> {
    let path = get_str(args, "path")?;
    let content = get_str(args, "content")?;
    host::workspace_append(path, content)
}

fn dispatch_list_dir(args: &serde_json::Value) -> Result<String, String> {
    let path = get_str(args, "path")?;
    host::workspace_list_dir(path)
}

fn dispatch_cron(args: &serde_json::Value) -> Result<String, String> {
    let action = get_str(args, "action")?;
    let payload = serde_json::to_string(args).unwrap_or_default();
    host::cron_store_op(action, &payload)
}

fn dispatch_recall(args: &serde_json::Value) -> Result<String, String> {
    let id = get_str(args, "id")?;
    let action = if id == "list" { "list" } else { "recall" };
    let payload = serde_json::to_string(args).unwrap_or_default();
    host::spill_store_op(action, &payload)
}

fn dispatch_message(args: &serde_json::Value) -> Result<String, String> {
    let text = get_str(args, "text")?;
    let target = args
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    host::send_message(target, text)
}

fn dispatch_web_search(args: &serde_json::Value) -> Result<String, String> {
    let query = get_str(args, "query")?;
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json",
        query.replace(' ', "+")
    );
    host::http_request("GET", &url, "{}", "")
}
