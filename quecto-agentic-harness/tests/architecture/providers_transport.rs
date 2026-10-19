//! #2597: `infrastructure/providers/transport/` holds the vendor-neutral
//! HTTP/SSE and attempt mechanics (wiki *Infrastructure*). What a vendor's
//! wire means is that vendor's `WireDialect`, beside its codec in
//! `anthropic/`, `openai/` or `codex/`. Every file of `transport/`, at any
//! depth and tests included, is held to an allowlist:
//! - every crate path it names, resolved (`super::`/`self::` from the
//!   file's place in the module tree; `use` leaves; paths in macro tokens),
//!   starts with an [`ALLOWED`] prefix, and a crate or module alias, which
//!   cannot be followed, is refused;
//! - no identifier, and no string literal outside documentation, names a
//!   vendor ([`VENDOR_WORDS`]).
use super::{dependency_scan, swarm_board_ports::crate_paths};
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;
use std::path::{Path, PathBuf};

const TRANSPORT: &str = "src/infrastructure/providers/transport";

/// The crate paths `transport/` may name: the inner layers, itself, the
/// provider root's own transport binding, and — for its tests' end-to-end
/// rigs only — the agent loop's tool registry and the vendor-neutral retry
/// decorator.
const ALLOWED: &[&str] = &[
    "crate::domain::",
    "crate::application::",
    "crate::infrastructure::providers::transport::",
    "crate::infrastructure::providers::AttemptTransportBinding",
    "crate::infrastructure::providers::ProviderBinding",
    "crate::infrastructure::providers::routing::retry::",
    "crate::infrastructure::tools::registry::",
];

/// The vendors, as they may appear in an identifier or a string: any case,
/// any position (`AnthropicQuirk`, `openai_wire`, `"OpenAI"`).
const VENDOR_WORDS: &[&str] = &["openai", "anthropic", "codex", "claude"];

fn allowed(path: &str) -> bool {
    path != dependency_scan::UNRESOLVABLE && ALLOWED.iter().any(|prefix| path.starts_with(prefix))
}

/// Every `.rs` file under `dir`, at any depth.
fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|error| panic!("read {dir:?}: {error}")) {
        let path = entry.expect("read an entry").path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

/// The module `file` is: its module path for a production module; for a
/// test file, the module of the file that declares it (by `#[path]` in the
/// same folder) plus the name it is declared as.
fn module_of(file: &Path) -> Vec<String> {
    let text = file.to_str().expect("a UTF-8 path");
    if !dependency_scan::test_only_file(text) {
        return dependency_scan::module_path(text);
    }
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .expect("a file name");
    let dir = file.parent().expect("a folder");
    let attribute = format!("#[path = \"{name}\"]");
    let declaring = std::fs::read_dir(dir)
        .expect("read the folder")
        .map(|entry| entry.expect("an entry").path())
        .find_map(|sibling| {
            let source = std::fs::read_to_string(&sibling).ok()?;
            let after = &source[source.find(&attribute)? + attribute.len()..];
            let declaration = after.lines().map(str::trim).find(|line| !line.is_empty())?;
            let module = declaration
                .trim_end_matches(';')
                .rsplit(' ')
                .next()?
                .to_owned();
            Some((sibling, module))
        });
    let (sibling, module) = declaring.unwrap_or_else(|| panic!("{text}: no file declares it"));
    let mut path = module_of(&sibling);
    path.push(module);
    path
}

/// The identifiers and string literals of `stream` that name a vendor,
/// outside documentation attributes (`#[doc = "…"]`, which comments are).
fn vendor_tokens(stream: TokenStream, found: &mut Vec<String>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for token in &tokens {
        match token {
            TokenTree::Group(group) => {
                // `#[doc = "…"]` and `#![doc = "…"]`: what comments are.
                let doc = group.delimiter() == Delimiter::Bracket
                    && matches!(group.stream().into_iter().next(), Some(TokenTree::Ident(i)) if i == "doc");
                if !doc {
                    vendor_tokens(group.stream(), found);
                }
            }
            TokenTree::Ident(ident) => {
                let word = ident.to_string().to_ascii_lowercase().replace('_', "");
                if VENDOR_WORDS.iter().any(|vendor| word.contains(vendor)) {
                    found.push(ident.to_string());
                }
            }
            TokenTree::Literal(literal) => {
                let text = literal.to_string().to_ascii_lowercase();
                let string = text.starts_with('"') || text.starts_with('r');
                if string && VENDOR_WORDS.iter().any(|vendor| text.contains(vendor)) {
                    found.push(literal.to_string());
                }
            }
            TokenTree::Punct(_) => {}
        }
    }
}

/// What `source`, the module `module`, breaks: each crate path outside
/// [`ALLOWED`] and each vendor token.
fn violations(source: &str, module: &[String]) -> Vec<String> {
    let mut found: Vec<String> = crate_paths(source, module)
        .into_iter()
        .filter(|path| !allowed(path))
        .collect();
    let file = syn::parse_file(source).expect("a crate source parses");
    vendor_tokens(file.to_token_stream(), &mut found);
    found
}

#[test]
fn transport_names_only_vendor_neutral_paths_and_no_vendor() {
    let mut files = Vec::new();
    rust_files(Path::new(TRANSPORT), &mut files);
    files.sort();
    let names: Vec<String> = files
        .iter()
        .filter_map(|file| file.file_name()?.to_str().map(str::to_owned))
        .collect();
    assert!(
        [
            "attempt_observer.rs",
            "attempt_profile.rs",
            "wire_dialect.rs"
        ]
        .iter()
        .all(|name| names.iter().any(|found| found == name))
            && names.iter().any(|name| name.ends_with("_tests.rs")),
        "the scan reads transport's production files and its tests: {names:?}"
    );
    let found: Vec<String> = files
        .iter()
        .flat_map(|file| {
            let source = std::fs::read_to_string(file).expect("read a source");
            violations(&source, &module_of(file))
                .into_iter()
                .map(move |hit| format!("{}: {hit}", file.display()))
        })
        .collect();
    assert!(
        found.is_empty(),
        "transport/ names a vendor or a path outside its allowlist; move it into that vendor's WireDialect: {found:#?}"
    );
}

/// Each way a vendor could be named from `transport/` is caught (#2605
/// review H1), and vendor-neutral code is not.
#[test]
fn the_scan_refuses_every_way_of_naming_a_vendor() {
    let here: Vec<String> = ["infrastructure", "providers", "transport", "wire_dialect"]
        .iter()
        .map(|segment| segment.to_string())
        .collect();
    for evasion in [
        "use crate::infrastructure::providers::openai as evade_a;",
        "use crate::infrastructure::providers::codex;",
        "use super::super::{anthropic as evade_g};",
        "use super::super::openai::openai_wire::OPENAI_WIRE;",
        "fn f(name: &str) -> bool { matches!(name, \"anthropic\" | \"OpenAI\") }",
        "enum EvadeD { OpenAi, Codex, Anthropic }",
        "struct AnthropicQuirk;",
        "fn g() -> u64 { evade_a::openai_wire::OPENAI_WIRE.output_bytes(&v) }",
        "fn h() { crate::infrastructure::providers::routing::provider_factory::make(); }",
        "use crate as root;",
        "use super::super::super as up;",
        "fn i() { tracing::info!(\"{}\", crate::infrastructure::providers::codex::X); }",
        "const NAME: &str = r\"Claude\";",
    ] {
        assert!(
            !violations(evasion, &here).is_empty(),
            "not caught: {evasion}"
        );
    }
    for neutral in [
        "use crate::domain::shared::error::DomainError;",
        "use super::sse_common::event_data;",
        "use crate::infrastructure::providers::transport::stream_idle::StreamIdle;",
        "/// Codex and OpenAI streams alike.\nfn j() {}",
        "fn k(vendor: &str) -> bool { vendor.is_empty() }",
        "const DONE: &str = \"[DONE]\";",
    ] {
        assert!(
            violations(neutral, &here).is_empty(),
            "refused: {neutral}: {:?}",
            violations(neutral, &here)
        );
    }
}
