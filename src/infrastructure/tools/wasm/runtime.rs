//! Wasmtime engine configuration and module cache.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use wasmtime::{Config, Engine};

/// Configuration for the WASM tool runtime.
#[derive(Debug, Clone)]
pub struct WasmRuntimeConfig {
    /// Maximum fuel (instruction budget) per tool execution.
    pub fuel_limit: u64,
    /// Maximum memory in bytes per tool execution.
    pub memory_limit: usize,
    /// Epoch tick interval for timeout interruption.
    pub epoch_tick_interval: Duration,
    /// Maximum execution time per tool call.
    pub execution_timeout: Duration,
    /// Maximum log entries per execution.
    pub max_log_entries: usize,
}

impl Default for WasmRuntimeConfig {
    fn default() -> Self {
        Self {
            fuel_limit: 10_000_000,
            memory_limit: 10 * 1024 * 1024, // 10 MB
            epoch_tick_interval: Duration::from_millis(500),
            execution_timeout: Duration::from_secs(30),
            max_log_entries: 1000,
        }
    }
}

/// A compiled WASM module cached for repeated instantiation.
pub struct PreparedModule {
    /// The compiled Wasmtime component.
    pub component: wasmtime::component::Component,
    /// Module name (tool name).
    pub name: String,
    /// Raw WASM bytes (kept for recompilation if needed).
    pub wasm_bytes: Vec<u8>,
}

impl std::fmt::Debug for PreparedModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedModule")
            .field("name", &self.name)
            .field("wasm_bytes_len", &self.wasm_bytes.len())
            .finish()
    }
}

/// The WASM tool runtime: manages the Wasmtime engine and module cache.
pub struct WasmToolRuntime {
    engine: Engine,
    config: WasmRuntimeConfig,
    modules: RwLock<HashMap<String, Arc<PreparedModule>>>,
    /// Handle to the epoch ticker thread (kept alive).
    _epoch_ticker: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for WasmToolRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let module_count = self.modules.read().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("WasmToolRuntime")
            .field("module_count", &module_count)
            .finish()
    }
}

impl WasmToolRuntime {
    /// Create a new WASM tool runtime with the given configuration.
    pub fn new(config: WasmRuntimeConfig) -> Result<Self, String> {
        let mut wasmtime_config = Config::new();
        wasmtime_config.consume_fuel(true);
        wasmtime_config.epoch_interruption(true);
        wasmtime_config.wasm_component_model(true);
        wasmtime_config.wasm_threads(false);
        wasmtime_config.debug_info(false);

        let engine =
            Engine::new(&wasmtime_config).map_err(|e| format!("failed to create engine: {e}"))?;

        // Start epoch ticker thread for timeout enforcement.
        let ticker_engine = engine.clone();
        let tick_interval = config.epoch_tick_interval;
        let ticker = std::thread::Builder::new()
            .name("wasm-epoch-ticker".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(tick_interval);
                    ticker_engine.increment_epoch();
                }
            })
            .map_err(|e| format!("failed to start epoch ticker: {e}"))?;

        Ok(Self {
            engine,
            config,
            modules: RwLock::new(HashMap::new()),
            _epoch_ticker: Some(ticker),
        })
    }

    /// Return a reference to the Wasmtime engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Return the runtime configuration.
    pub fn config(&self) -> &WasmRuntimeConfig {
        &self.config
    }

    /// Compile and cache a WASM module. Returns the cached module if already
    /// present.
    pub fn prepare(&self, name: &str, wasm_bytes: &[u8]) -> Result<Arc<PreparedModule>, String> {
        // Check cache first.
        if let Some(module) = self.get(name) {
            return Ok(module);
        }

        let component = wasmtime::component::Component::new(&self.engine, wasm_bytes)
            .map_err(|e| format!("failed to compile WASM module '{name}': {e}"))?;

        let prepared = Arc::new(PreparedModule {
            component,
            name: name.to_string(),
            wasm_bytes: wasm_bytes.to_vec(),
        });

        self.modules
            .write()
            .map_err(|e| format!("module cache lock poisoned: {e}"))?
            .insert(name.to_string(), prepared.clone());

        Ok(prepared)
    }

    /// Look up a cached module by name.
    pub fn get(&self, name: &str) -> Option<Arc<PreparedModule>> {
        self.modules.read().ok()?.get(name).cloned()
    }

    /// Remove a module from the cache.
    pub fn remove(&self, name: &str) -> Option<Arc<PreparedModule>> {
        self.modules.write().ok()?.remove(name)
    }

    /// List all cached module names.
    pub fn list(&self) -> Vec<String> {
        self.modules
            .read()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Clear all cached modules.
    pub fn clear(&self) {
        if let Ok(mut modules) = self.modules.write() {
            modules.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_creation_with_defaults() {
        let rt = WasmToolRuntime::new(WasmRuntimeConfig::default());
        assert!(rt.is_ok());
    }

    #[test]
    fn test_runtime_config_defaults() {
        let config = WasmRuntimeConfig::default();
        assert_eq!(config.fuel_limit, 10_000_000);
        assert_eq!(config.memory_limit, 10 * 1024 * 1024);
        assert_eq!(config.max_log_entries, 1000);
    }

    #[test]
    fn test_empty_module_cache() {
        let rt = WasmToolRuntime::new(WasmRuntimeConfig::default()).unwrap();
        assert!(rt.list().is_empty());
        assert!(rt.get("nonexistent").is_none());
    }

    #[test]
    fn test_prepare_invalid_wasm_returns_error() {
        let rt = WasmToolRuntime::new(WasmRuntimeConfig::default()).unwrap();
        let result = rt.prepare("bad", b"not valid wasm");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed to compile"));
    }

    #[test]
    fn test_remove_returns_none_for_missing() {
        let rt = WasmToolRuntime::new(WasmRuntimeConfig::default()).unwrap();
        assert!(rt.remove("nonexistent").is_none());
    }

    #[test]
    fn test_clear_empties_cache() {
        let rt = WasmToolRuntime::new(WasmRuntimeConfig::default()).unwrap();
        // Cache is already empty, just verify clear doesn't panic.
        rt.clear();
        assert!(rt.list().is_empty());
    }

    #[test]
    fn test_debug_format() {
        let rt = WasmToolRuntime::new(WasmRuntimeConfig::default()).unwrap();
        let debug = format!("{:?}", rt);
        assert!(debug.contains("WasmToolRuntime"));
        assert!(debug.contains("module_count"));
    }

    #[test]
    fn test_engine_has_expected_config() {
        let rt = WasmToolRuntime::new(WasmRuntimeConfig::default()).unwrap();
        // The engine exists and was configured — we verify by attempting
        // to create a component (which requires component-model enabled).
        let minimal_wasm = wat_to_component_bytes(MINIMAL_COMPONENT_WAT);
        let component = wasmtime::component::Component::new(rt.engine(), &minimal_wasm);
        assert!(component.is_ok(), "engine should support component model");
    }

    #[test]
    fn test_prepare_and_get_module() {
        let rt = WasmToolRuntime::new(WasmRuntimeConfig::default()).unwrap();
        let wasm = wat_to_component_bytes(MINIMAL_COMPONENT_WAT);
        let module = rt.prepare("test_tool", &wasm);
        assert!(module.is_ok());
        assert_eq!(module.unwrap().name, "test_tool");
        assert!(rt.get("test_tool").is_some());
        assert_eq!(rt.list().len(), 1);
    }

    #[test]
    fn test_prepare_returns_cached_on_second_call() {
        let rt = WasmToolRuntime::new(WasmRuntimeConfig::default()).unwrap();
        let wasm = wat_to_component_bytes(MINIMAL_COMPONENT_WAT);
        let first = rt.prepare("tool", &wasm).unwrap();
        let second = rt.prepare("tool", &wasm).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn test_remove_module() {
        let rt = WasmToolRuntime::new(WasmRuntimeConfig::default()).unwrap();
        let wasm = wat_to_component_bytes(MINIMAL_COMPONENT_WAT);
        rt.prepare("tool", &wasm).unwrap();
        assert!(rt.get("tool").is_some());
        let removed = rt.remove("tool");
        assert!(removed.is_some());
        assert!(rt.get("tool").is_none());
    }

    /// Minimal valid WAT component for testing compilation.
    const MINIMAL_COMPONENT_WAT: &str = r#"
        (component
            (core module $m
                (func (export "memory") (result i32) (i32.const 0))
                (memory (export "mem") 1)
            )
        )
    "#;

    /// Convert WAT text to WASM binary bytes using wasmtime's built-in parser.
    fn wat_to_component_bytes(wat: &str) -> Vec<u8> {
        wat::parse_str(wat).expect("valid WAT")
    }
}
