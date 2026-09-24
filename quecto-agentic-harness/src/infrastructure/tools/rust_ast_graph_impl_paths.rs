use super::{Symbol, crate_directory, is_rust_ident_continue};

// Associate only syntactically local paths with declarations in the same crate.
// Imports and aliases require compiler resolution; treating terminal identifiers as
// identity creates false edges between unrelated crates and sibling modules.
pub(super) fn local_impl_path_matches(
    implementation: &Symbol,
    path: &str,
    target: &Symbol,
) -> bool {
    match (
        crate_directory(&implementation.location.file),
        crate_directory(&target.location.file),
    ) {
        (implementation_crate, target_crate) if implementation_crate == target_crate => {}
        _ => return false,
    }
    let path = path.trim();
    let path = path.strip_prefix('&').unwrap_or(path).trim();
    let path = path.strip_prefix("mut ").unwrap_or(path).trim();
    let path = path.split('<').next().unwrap_or(path).trim();
    // Only unambiguous identifier paths are candidates; do not invent edges
    // for qualified imports, associated types or expressions.
    if path.split("::").all(|segment| {
        segment
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_alphabetic())
            && segment.chars().all(is_rust_ident_continue)
    }) {
        // Continue with syntactically valid local segments.
    } else {
        return false;
    }
    let module = implementation.module.as_str();
    let mut base = if module == "crate" { "" } else { module };
    let mut remainder = path;
    if let Some(rest) = path.strip_prefix("crate::") {
        base = "";
        remainder = rest;
    } else if let Some(rest) = path.strip_prefix("self::") {
        remainder = rest;
    } else if path.starts_with("super::") {
        while let Some(rest) = remainder.strip_prefix("super::") {
            base = base.rsplit_once("::").map_or("", |(parent, _)| parent);
            remainder = rest;
        }
    } else if path.starts_with("::") {
        return false;
    }
    let resolved = if base.is_empty() {
        remainder.to_string()
    } else {
        format!("{base}::{remainder}")
    };
    resolved == target.qualified_path
}
