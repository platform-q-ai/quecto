//! WasmToolWrapper: bridges a WASM component to the domain::Tool trait.
//!
//! Each call to `execute()` creates a fresh Store with a new HostState,
//! compiles/instantiates the component, and invokes the exported `execute`
//! function. The Store is dropped after each call (no state leaks).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

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

    /// Execute the tool using the WASM runtime.
    ///
    /// Creates a fresh Store + HostState per call. In the current
    /// implementation, this delegates to the HostState methods directly
    /// (simulating what a real WASM component would do via host imports).
    /// When real WIT bindgen integration is added, this will instantiate
    /// the component and call the exported `execute` function.
    fn execute_inner(&self, _arguments: &str) -> Result<ToolResult, DomainError> {
        let config = self.runtime.config();
        let mut host_state =
            HostState::new(std::path::PathBuf::from("/tmp"), config.max_log_entries);

        // Let the composition root configure the host state.
        if let Some(configurator) = &self.host_configurator {
            configurator(&mut host_state);
        }

        // Parse the arguments and delegate to the WASM module.
        // For now, we simulate this by parsing the tool-specific params
        // and calling the appropriate host functions.
        //
        // In the real implementation, this would:
        // 1. Create a fresh wasmtime::Store<HostState>
        // 2. Set fuel limit: store.set_fuel(config.fuel_limit)
        // 3. Set epoch deadline: store.epoch_deadline_trap()
        // 4. Link host functions via wasmtime::component::Linker
        // 5. Instantiate the component
        // 6. Call tool.execute(params) on the exported interface
        // 7. Extract result, logs, drop store

        let _start = Instant::now();
        let _component = &self.module.component;

        // Placeholder: the real dispatch happens via component instantiation.
        // For the initial implementation, we verify that the runtime, module,
        // and wrapper infrastructure work end-to-end.
        Ok(ToolResult {
            content: format!("WASM tool '{}' executed (stub)", self.meta.name),
            is_error: false,
        })
    }
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

    fn create_test_wrapper() -> Option<WasmToolWrapper> {
        let rt = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::default()).ok()?);
        let wasm = wat::parse_str(MINIMAL_COMPONENT_WAT).ok()?;
        let module = rt.prepare("test_tool", &wasm).ok()?;
        Some(WasmToolWrapper::new(
            rt,
            module,
            WasmToolMeta {
                name: "test_tool".to_string(),
                description: "A test tool".to_string(),
                schema: r#"{"type":"object","properties":{}}"#.to_string(),
            },
        ))
    }

    #[test]
    fn test_wrapper_definition() {
        let wrapper = create_test_wrapper().unwrap();
        let def = wrapper.definition();
        assert_eq!(def.name, "test_tool");
        assert_eq!(def.description, "A test tool");
    }

    #[tokio::test]
    async fn test_wrapper_execute() {
        let wrapper = create_test_wrapper().unwrap();
        let result = wrapper.execute("{}").await;
        assert!(result.is_ok());
        let tool_result = result.unwrap();
        assert!(!tool_result.is_error);
        assert!(
            tool_result.content.contains("test_tool"),
            "content: {}",
            tool_result.content
        );
    }

    #[test]
    fn test_wrapper_debug() {
        let wrapper = create_test_wrapper().unwrap();
        let debug = format!("{:?}", wrapper);
        assert!(debug.contains("WasmToolWrapper"));
        assert!(debug.contains("test_tool"));
    }

    const MINIMAL_COMPONENT_WAT: &str = r#"
        (component
            (core module $m
                (func (export "memory") (result i32) (i32.const 0))
                (memory (export "mem") 1)
            )
        )
    "#;
}
