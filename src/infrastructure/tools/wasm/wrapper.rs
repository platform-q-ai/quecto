//! WasmToolWrapper: bridges a WASM component to the domain::Tool trait.
//!
//! Each call to `execute()` creates a fresh Store with a new HostState,
//! instantiates the WASM component, links host functions via the WIT
//! interface, and calls the exported `execute` function. The Store is
//! dropped after each call — no state carries between invocations.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use wasmtime::Store;
use wasmtime::component::Linker;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

use super::bindings::SandboxedTool;
use super::host::HostState;
use super::runtime::{PreparedModule, WasmToolRuntime};

/// A callback that can configure the HostState before each tool execution.
/// Used by the composition root to inject real trait-port adapters.
pub type HostConfigurator = Arc<dyn Fn(&mut HostState) + Send + Sync>;

/// Metadata for constructing a WasmToolWrapper.
pub struct WasmToolMeta {
    /// Tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for parameters.
    pub schema: String,
}

/// Wraps a compiled WASM component as a domain::Tool.
///
/// The component is instantiated fresh on each `execute()` call with a
/// new `Store<HostState>`. Fuel metering and epoch interruption enforce
/// resource limits. Host functions are linked via `SandboxedTool::add_to_linker`.
pub struct WasmToolWrapper {
    runtime: Arc<WasmToolRuntime>,
    module: Arc<PreparedModule>,
    meta: WasmToolMeta,
    /// Optional callback to configure HostState before each execution.
    host_configurator: Option<HostConfigurator>,
}

impl std::fmt::Debug for WasmToolWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmToolWrapper")
            .field("name", &self.meta.name)
            .finish()
    }
}

impl WasmToolWrapper {
    /// Create a new wrapper for a compiled WASM tool module.
    pub fn new(
        runtime: Arc<WasmToolRuntime>,
        module: Arc<PreparedModule>,
        meta: WasmToolMeta,
    ) -> Self {
        Self {
            runtime,
            module,
            meta,
            host_configurator: None,
        }
    }

    /// Set a callback to configure the HostState before each execution.
    pub fn with_host_configurator(mut self, configurator: HostConfigurator) -> Self {
        self.host_configurator = Some(configurator);
        self
    }

    /// Execute the tool via real WASM component instantiation.
    ///
    /// 1. Create a fresh `Store<HostState>` with fuel + epoch limits
    /// 2. Link host functions via `SandboxedTool::add_to_linker`
    /// 3. Instantiate the WASM component
    /// 4. Call the exported `execute(params)` function
    /// 5. Drop the Store (no state leaks)
    fn execute_inner(&self, arguments: &str) -> Result<ToolResult, DomainError> {
        let config = self.runtime.config();

        // Build HostState for this invocation.
        let mut host_state =
            HostState::new(std::path::PathBuf::from("/tmp"), config.max_log_entries);
        host_state.set_memory_limit(config.memory_limit);
        if let Some(configurator) = &self.host_configurator {
            configurator(&mut host_state);
        }

        // Create Store with fuel and epoch enforcement.
        let mut store = Store::new(self.runtime.engine(), host_state);
        store.limiter(|state| state.store_limits_mut());
        store
            .set_fuel(config.fuel_limit)
            .map_err(|e| DomainError::Tool(format!("set fuel: {e}")))?;
        store.epoch_deadline_trap();
        store.set_epoch_deadline(epoch_deadline_ticks(config));

        // Link WASI host imports (required by wasm32-wasip2 components).
        let mut linker: Linker<HostState> = Linker::new(self.runtime.engine());
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|e| DomainError::Tool(format!("link wasi: {e}")))?;

        // Link our custom host functions from the WIT interface.
        SandboxedTool::add_to_linker::<_, wasmtime::component::HasSelf<HostState>>(
            &mut linker,
            |state| state,
        )
        .map_err(|e| DomainError::Tool(format!("link host: {e}")))?;

        // Instantiate the WASM component.
        let instance = SandboxedTool::instantiate(&mut store, &self.module.component, &linker)
            .map_err(|e| DomainError::Tool(format!("instantiate: {e}")))?;

        // Inject __tool field so the guest dispatch knows which tool.
        let params = inject_tool_name(arguments, &self.meta.name);

        // Call the exported execute function across the WASM boundary.
        let result = instance
            .quecto_tools_tool()
            .call_execute(&mut store, &params)
            .map_err(|e| DomainError::Tool(format!("call execute: {e}")))?;

        match result {
            Ok(content) => Ok(ToolResult {
                content,
                is_error: false,
            }),
            Err(content) => Ok(ToolResult {
                content,
                is_error: true,
            }),
        }
    }
}

/// Inject `__tool` into the JSON arguments so the guest knows which
/// tool to dispatch.
fn inject_tool_name(arguments: &str, tool_name: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(serde_json::Value::Object(mut map)) => {
            map.insert(
                "__tool".to_string(),
                serde_json::Value::String(tool_name.to_string()),
            );
            serde_json::to_string(&map).unwrap_or_else(|_| arguments.to_string())
        }
        _ => {
            // If args aren't a JSON object, wrap them.
            serde_json::json!({ "__tool": tool_name }).to_string()
        }
    }
}

fn epoch_deadline_ticks(
    config: &crate::infrastructure::tools::wasm::runtime::WasmRuntimeConfig,
) -> u64 {
    let tick_ns = config.epoch_tick_interval.as_nanos().max(1);
    let timeout_ns = config.execution_timeout.as_nanos();
    let ticks = timeout_ns.div_ceil(tick_ns);
    ticks.max(1) as u64
}

impl Tool for WasmToolWrapper {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.meta.name.clone(),
            description: self.meta.description.clone(),
            parameters_schema: self.meta.schema.clone(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move { self.execute_inner(&args) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::tools::wasm::runtime::WasmRuntimeConfig;

    /// The real guest component bytes, compiled from guest/src/lib.rs.
    const GUEST_WASM: &[u8] = include_bytes!("../../../../guest/quecto_wasm_guest.wasm");

    fn create_real_wrapper(name: &str) -> WasmToolWrapper {
        let rt = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::default()).unwrap());
        let module = rt.prepare(name, GUEST_WASM).unwrap();
        WasmToolWrapper::new(
            rt,
            module,
            WasmToolMeta {
                name: name.to_string(),
                description: format!("{name} tool"),
                schema: r#"{"type":"object"}"#.to_string(),
            },
        )
    }

    #[test]
    fn test_wrapper_definition() {
        let wrapper = create_real_wrapper("read_file");
        let def = wrapper.definition();
        assert_eq!(def.name, "read_file");
    }

    #[tokio::test]
    async fn test_wrapper_execute_read_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path().to_path_buf();
        std::fs::write(ws.join("test.txt"), "hello from wasm").unwrap();

        let wrapper = create_real_wrapper("read_file").with_host_configurator(Arc::new(
            move |host: &mut HostState| {
                host.workspace = ws.clone();
            },
        ));

        let result = wrapper.execute(r#"{"path":"test.txt"}"#).await.unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert_eq!(result.content, "hello from wasm");
    }

    #[tokio::test]
    async fn test_wrapper_execute_write_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path().to_path_buf();
        let ws2 = ws.clone();

        let wrapper = create_real_wrapper("write_file").with_host_configurator(Arc::new(
            move |host: &mut HostState| {
                host.workspace = ws2.clone();
            },
        ));

        let result = wrapper
            .execute(r#"{"path":"out.txt","content":"wasm wrote this"}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);

        let content = std::fs::read_to_string(ws.join("out.txt")).unwrap();
        assert_eq!(content, "wasm wrote this");
    }

    #[tokio::test]
    async fn test_wrapper_execute_unknown_tool() {
        let wrapper = create_real_wrapper("nonexistent_tool");
        let result = wrapper.execute(r#"{}"#).await.unwrap();
        assert!(result.is_error);
        assert!(
            result.content.contains("unknown tool"),
            "content: {}",
            result.content
        );
    }

    #[test]
    fn test_wrapper_debug() {
        let wrapper = create_real_wrapper("read_file");
        let debug = format!("{:?}", wrapper);
        assert!(debug.contains("WasmToolWrapper"));
        assert!(debug.contains("read_file"));
    }

    #[test]
    fn test_inject_tool_name() {
        let result = inject_tool_name(r#"{"path":"test.txt"}"#, "read_file");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["__tool"], "read_file");
        assert_eq!(parsed["path"], "test.txt");
    }

    #[test]
    fn test_inject_tool_name_invalid_json() {
        let result = inject_tool_name("not json", "read_file");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["__tool"], "read_file");
    }

    #[test]
    fn test_epoch_deadline_ticks_minimum_one() {
        let cfg = WasmRuntimeConfig {
            execution_timeout: std::time::Duration::from_nanos(1),
            epoch_tick_interval: std::time::Duration::from_secs(1),
            ..WasmRuntimeConfig::default()
        };
        assert_eq!(epoch_deadline_ticks(&cfg), 1);
    }
}
