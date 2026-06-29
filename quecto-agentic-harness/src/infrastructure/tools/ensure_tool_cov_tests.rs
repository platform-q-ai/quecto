//! Coverage-focused unit tests for `ensure_tool.rs`.
//!
//! These exercise the pure PATH-resolution helper `which_path` without spawning
//! a subprocess (unlike `which`, which runs `<name> --version`). Download,
//! offline (`QUECTO_OFFLINE`), and `which` branches require network / a real
//! process / env mutation and are covered by the existing inline + BDD tests.

use super::*;

#[test]
fn which_path_finds_existing_binary_on_path() {
    // `sh` exists on every supported test host's PATH, so the directory scan
    // returns the concrete file path (the is_file() match arm).
    let resolved = which_path("sh").expect("sh should resolve");
    assert!(resolved.ends_with("sh"), "got: {resolved:?}");
    assert!(
        resolved.is_file(),
        "resolved path should be a file: {resolved:?}"
    );
}

#[test]
fn which_path_falls_back_to_bare_name_when_not_found() {
    // A name that is not on PATH falls through the directory scan to the
    // bare-name fallback (Some(PathBuf::from(name))).
    let resolved =
        which_path("definitely-not-a-real-binary-xyz-123").expect("fallback is always Some");
    assert_eq!(
        resolved,
        PathBuf::from("definitely-not-a-real-binary-xyz-123")
    );
}

#[tokio::test]
async fn ensure_tool_wrapper_rejects_unknown_tool_without_io() {
    // Exercises the public `ensure_tool` wrapper (which resolves the default
    // cache dir via `tools_cache_dir()` and delegates to
    // `ensure_tool_with_cache`). An unknown tool fails at `find_config` before
    // any PATH lookup, network, or filesystem access — fully deterministic.
    let err = ensure_tool("definitely-not-a-tool-xyz")
        .await
        .expect_err("unknown tool must error");
    assert!(err.to_string().contains("Unknown tool"), "got: {err}");
}
