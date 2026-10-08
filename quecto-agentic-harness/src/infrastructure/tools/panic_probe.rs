//! A fault-injection tool for `test-support` builds only (#2192): proves,
//! against a real `quecto` process, that a panicking tool costs one call
//! while any other panic still ends the process. Registered only when a
//! `test-support` build runs with `QUECTO_TEST_PANIC_PROBE=1`; a
//! production build does not compile it.
//!
//! Arguments: `{"where": "tool"}` panics inside the call (contained, with a
//! `str` slicing panic like the one that killed agents in #2192);
//! `{"where": "outside"}` panics on a thread the call starts but does not
//! carry its scope to (fatal: the process aborts).
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};

/// The environment switch that registers the probe.
pub const PANIC_PROBE_ENV: &str = "QUECTO_TEST_PANIC_PROBE";

/// The probe's tool name.
pub const PANIC_PROBE_TOOL: &str = "panic_probe";

#[derive(Debug, Default)]
pub struct PanicProbeTool;

impl Tool for PanicProbeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: PANIC_PROBE_TOOL.into(),
            description: "Test-support fault injection: panics where asked.".into(),
            parameters_schema: r#"{"type":"object","properties":{"where":{"type":"string","enum":["tool","outside"]}},"required":["where"]}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let place = serde_json::from_str::<serde_json::Value>(arguments)
            .ok()
            .and_then(|value| value["where"].as_str().map(str::to_owned))
            .unwrap_or_default();
        Box::pin(async move {
            tokio::task::yield_now().await;
            match place.as_str() {
                "tool" => {
                    let text = "’’ab’";
                    let cut = text.len() - 2;
                    Ok(ok(&text[..cut]))
                }
                "outside" => {
                    let _ =
                        std::thread::spawn(|| panic!("panic_probe: a panic outside any tool call"))
                            .join();
                    Ok(ok("the process survived a fatal panic"))
                }
                other => Err(DomainError::Tool(format!(
                    "panic_probe: 'where' must be \"tool\" or \"outside\", got {other:?}"
                ))),
            }
        })
    }
}

fn ok(content: &str) -> ToolResult {
    ToolResult {
        content: content.to_string(),
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

/// Register the probe when the environment asks for it (value `1` only).
pub fn register_if_requested(registry: &mut super::registry::ToolRegistryImpl) {
    if std::env::var(PANIC_PROBE_ENV).is_ok_and(|value| value == "1") {
        let registered = registry.register(Arc::new(PanicProbeTool));
        debug_assert!(registered, "the panic probe registers once");
    }
}

#[cfg(test)]
#[path = "panic_probe_tests.rs"]
mod tests;
