#![allow(dead_code)]

//! Shared conformance assertions for epic #1193's "no duplicate authority"
//! acceptance criteria (slice 6, issue #1576).
//!
//! One implementation of the source-tree scans, canonical-capability checks,
//! and contributor-docs structure checks, reused by the BDD convergence
//! steps, the `issue_1193_completion` acceptance guard, and the consumer
//! contract tests — so the conformance rules themselves have a single
//! authority too.

use std::fs;
use std::path::{Path, PathBuf};

pub fn harness_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The interface-level catalogue composition bridges slices 2/4 parked in
/// `interface/cli`; slice 6 removes them (composition lives in the shared
/// interface composition root, `interface/catalogue_runtime.rs`).
pub const LEGACY_BRIDGE_MODULES: [&str; 2] = [
    "src/interface/cli/catalogue_bridge.rs",
    "src/interface/cli/catalogue_refresh_bridge.rs",
];

/// The production portion (everything before a trailing `#[cfg(test)]`
/// module marker) of every non-test `.rs` file under `dir`, as
/// `(path, content)` pairs. `_tests.rs` companions are skipped entirely.
pub fn production_sources(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).expect("readable source dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            out.extend(production_sources(&path));
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
            continue;
        }
        let raw = fs::read_to_string(&path).expect("readable source file");
        out.push((
            path.display().to_string(),
            raw.split("#[cfg(test)]").next().unwrap().to_string(),
        ));
    }
    out
}

/// Legacy bridge modules still present on disk (empty = converged).
pub fn legacy_bridge_modules_present() -> Vec<String> {
    LEGACY_BRIDGE_MODULES
        .into_iter()
        .filter(|legacy| harness_root().join(legacy).exists())
        .map(str::to_string)
        .collect()
}

/// "No catalogue parsing/merging in interface": only infrastructure source
/// adapters may see the models.json wire format and the legacy registry
/// parser that reads it.
pub fn model_registry_readers(interface_sources: &[(String, String)]) -> Vec<String> {
    offenders(interface_sources, "infrastructure::model_registry")
}

/// Paths in `sources` whose production portion contains `needle`.
fn offenders(sources: &[(String, String)], needle: &str) -> Vec<String> {
    sources
        .iter()
        .filter(|(_, content)| content.contains(needle))
        .map(|(path, _)| path.clone())
        .collect()
}

/// The provider/model-name reasoning-effort heuristic may survive only as
/// the domain rule seeding canonical metadata (`ModelCapabilities::
/// effort_vocabulary_for`); interface and infrastructure consumers project
/// the canonical vocabulary instead of re-deriving one.
pub fn effort_name_inference_sites(layer_sources: &[(String, String)]) -> Vec<String> {
    offenders(layer_sources, "levels_for_model")
}

/// "No canonical types in infrastructure": infrastructure adapters map wire
/// formats INTO the domain's canonical catalogue types — they must never
/// define competing types of the same shape. (Referencing `domain::catalogue`
/// is the intended source-adapter boundary; redefining its types is not.)
pub fn canonical_type_redefinitions(infrastructure_sources: &[(String, String)]) -> Vec<String> {
    const CANONICAL: [&str; 7] = [
        "CatalogueSnapshot",
        "CatalogueEntry",
        "ModelCapabilities",
        "ModelDescriptor",
        "ProviderDescriptor",
        "ModelRef",
        "AvailabilityStatus",
    ];
    let mut found = Vec::new();
    for (path, content) in infrastructure_sources {
        for name in CANONICAL {
            for keyword in ["struct", "enum"] {
                if content.contains(&format!("{keyword} {name}")) {
                    found.push(format!("{path}: {keyword} {name}"));
                }
            }
        }
    }
    found
}

/// The built-in source's entries, loaded through the same `CatalogueSource`
/// port production resolution uses — the behavioral form of "canonical
/// capabilities declare an effort vocabulary" (a comment mentioning the
/// field cannot satisfy this).
pub fn builtin_entries() -> Vec<quecto::domain::catalogue::CatalogueEntry> {
    use quecto::application::catalogue::CatalogueSource as _;
    quecto::infrastructure::catalogue_registry::BuiltinCatalogueSource
        .load()
        .expect("builtin source loads")
        .entries
}

/// Built-in entries whose canonical capabilities carry NO effort vocabulary
/// (empty = every entry declares one). Panics if the builtin catalogue
/// itself is empty, which would make the check vacuous.
pub fn builtin_entries_missing_effort_vocabulary() -> Vec<String> {
    let entries = builtin_entries();
    assert!(!entries.is_empty(), "builtin catalogue is empty");
    entries
        .iter()
        .filter(|entry| entry.model.capabilities.effort_levels.is_empty())
        .map(|entry| entry.model.reference.qualified_id())
        .collect()
}

pub fn contributor_docs() -> String {
    fs::read_to_string(harness_root().join("../docs/runtime-models-providers.md"))
        .expect("contributor documentation exists")
}

/// The body of the markdown section opened by `heading` (exclusive), up to
/// the next same-or-higher-level heading — so assertions bind to the section
/// that owns a topic, not to stray mentions elsewhere in the document.
pub fn doc_section<'a>(docs: &'a str, heading: &str) -> &'a str {
    let start = docs
        .find(heading)
        .unwrap_or_else(|| panic!("missing documentation section: {heading}"))
        + heading.len();
    let level = heading.chars().take_while(|c| *c == '#').count();
    let end = (2..=level)
        .filter_map(|l| docs[start..].find(&format!("\n{} ", "#".repeat(l))))
        .min()
        .unwrap_or(docs.len() - start);
    &docs[start..start + end]
}

/// Structural contributor-docs contract: a layer-ownership map naming every
/// layer inside its own section, one section per contribution path the issue
/// requires, and an explicit warning against creating another authority.
/// The contribution-path sections the issue requires the contributor docs
/// to carry, one heading per path.
pub const REQUIRED_DOC_SECTIONS: [&str; 6] = [
    "## Source precedence",
    "## Add or change domain metadata",
    "## Add a catalogue source",
    "### Add a provider on an existing transport",
    "## Add a new transport or authentication flow",
    "### User overrides",
];

pub fn has_doc_section(docs: &str, heading: &str) -> bool {
    docs.contains(heading)
}

pub fn assert_contributor_docs_structure(docs: &str) {
    let ownership = doc_section(docs, "## Layer ownership").to_ascii_lowercase();
    for layer in ["domain", "application", "infrastructure", "interface"] {
        assert!(
            ownership.contains(&format!("`{layer}`")),
            "layer ownership map missing layer: {layer}"
        );
    }
    assert!(
        ownership.contains("another authority"),
        "layer ownership map must warn against creating another authority"
    );
    for heading in REQUIRED_DOC_SECTIONS {
        assert!(
            has_doc_section(docs, heading),
            "missing documentation section: {heading}"
        );
    }
    assert!(
        doc_section(docs, "## Add a new transport or authentication flow")
            .contains("another registry"),
        "new-transport guidance must forbid introducing another registry"
    );
}
