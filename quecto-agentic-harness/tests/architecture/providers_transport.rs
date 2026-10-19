//! #2597: `infrastructure/providers/transport/` holds the vendor-neutral
//! HTTP/SSE and attempt mechanics (wiki *Infrastructure*). What a vendor's
//! wire means is that vendor's `WireDialect`, beside its codec in
//! `anthropic/`, `openai/` or `codex/`. No production line of `transport/`
//! names a vendor module or matches on a vendor.
use super::teardown_authority::production_code;

const TRANSPORT: &str = "src/infrastructure/providers/transport";

/// The provider folders that are vendor codecs.
const VENDOR_MODULES: &[&str] = &["anthropic", "openai", "codex"];

/// Every production file of `transport/`.
fn transport_files() -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(TRANSPORT)
        .unwrap_or_else(|error| panic!("read {TRANSPORT}: {error}"))
        .map(|entry| entry.expect("read an entry").file_name())
        .map(|name| name.into_string().expect("a UTF-8 name"))
        .filter(|name| name.ends_with(".rs") && !name.ends_with("_tests.rs"))
        .map(|name| format!("{TRANSPORT}/{name}"))
        .collect();
    files.sort();
    files
}

/// Whether `line` holds `word` as a whole identifier, followed by `then`.
fn names(line: &str, word: &str, then: &str) -> bool {
    line.match_indices(word).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = &line[at + word.len()..];
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            && after.starts_with(then)
            && !(then.is_empty()
                && after
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_'))
    })
}

/// The production lines of `transport/` that name a vendor module or a
/// `Vendor` type.
fn vendor_lines(files: &[String]) -> Vec<String> {
    files
        .iter()
        .flat_map(|file| {
            production_code(file)
                .into_iter()
                .filter(|(_, line)| {
                    VENDOR_MODULES
                        .iter()
                        .any(|vendor| names(line, vendor, "::"))
                        || names(line, "Vendor", "")
                })
                .map(move |(number, line)| format!("{file}:{number}: {}", line.trim()))
        })
        .collect()
}

#[test]
fn transport_names_no_vendor_module_and_matches_on_no_vendor() {
    let files = transport_files();
    assert!(
        files
            .iter()
            .any(|file| file.ends_with("/attempt_observer.rs"))
            && files
                .iter()
                .any(|file| file.ends_with("/attempt_profile.rs")),
        "the scan reads the attempt observer and profile: {files:?}"
    );
    let found = vendor_lines(&files);
    assert!(
        found.is_empty(),
        "transport/ names a vendor; move it into that vendor's WireDialect: {found:#?}"
    );
}

#[test]
fn the_vendor_scan_sees_a_vendor_path_and_a_vendor_enum() {
    for line in [
        "use crate::infrastructure::providers::openai::openai_sse;",
        "super::super::codex::RESPONSES_CUT_SHORT",
        "(anthropic::ANTHROPIC_CUT_SHORT)",
        "pub enum Vendor {",
        "match self.vendor { Vendor::Codex => 0 }",
    ] {
        assert!(
            VENDOR_MODULES
                .iter()
                .any(|vendor| names(line, vendor, "::"))
                || names(line, "Vendor", ""),
            "{line}"
        );
    }
    for line in [
        "let vendor = 1;",
        "fn openai_wire() {}",
        "VendorNeutral",
        "my_codex::x",
        "\"OpenAI\"",
    ] {
        assert!(
            !(VENDOR_MODULES
                .iter()
                .any(|vendor| names(line, vendor, "::"))
                || names(line, "Vendor", "")),
            "{line}"
        );
    }
}
