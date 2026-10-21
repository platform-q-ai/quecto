pub mod model_refusal;
pub mod provider_factory;
pub mod provider_runtime;
pub mod provider_runtime_admission;
pub mod refreshable;
pub mod retry;
pub mod router;

// Cross-vendor suites: each drives the OpenAI, Codex and Anthropic providers
// through the shared stream bounds or request adapters. `transport/` may
// name no vendor (#2597), so they live with the vendor-assembling folder.
#[cfg(test)]
mod vendor_stream_bounds_end_2249_tests;
#[cfg(test)]
pub(crate) mod vendor_stream_bounds_idle_tests;
#[cfg(test)]
mod vendor_stream_bounds_output_cap_eof_tests;
#[cfg(test)]
mod vendor_stream_bounds_output_cap_tests;
#[cfg(test)]
mod vendor_stream_bounds_progress_tests;
#[cfg(test)]
mod vendor_tool_schema_tests;
