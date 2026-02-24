//! Wasmtime `bindgen!` generated bindings for the `sandboxed-tool` WIT world.
//!
//! This module provides:
//! - A `Host` trait matching the `quecto:tools/host` WIT imports
//! - Typed export wrappers for the `quecto:tools/tool` WIT exports
//! - `add_to_linker` to wire host functions into a wasmtime Linker
//!
//! The `Host` trait is implemented on `HostState` in `host.rs`.

wasmtime::component::bindgen!({
    world: "sandboxed-tool",
    path: "wit/tool.wit",
});

#[cfg(test)]
mod tests {
    use super::super::host::HostState;
    use super::*;
    use wasmtime::component::Linker;

    /// Verify add_to_linker compiles with our HostState.
    #[test]
    fn test_add_to_linker_compiles() {
        fn wire(linker: &mut Linker<HostState>) {
            SandboxedTool::add_to_linker::<HostState, wasmtime::component::HasSelf<HostState>>(
                linker,
                |s| s,
            )
            .unwrap();
        }
        // Just prove the function compiles; don't call it.
        let _ = wire;
    }
}
