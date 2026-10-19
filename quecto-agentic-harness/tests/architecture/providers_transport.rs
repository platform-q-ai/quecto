//! #2597: `infrastructure/providers/transport/` holds the vendor-neutral
//! HTTP/SSE and attempt mechanics (wiki *Infrastructure*). What a vendor's
//! wire means is that vendor's `WireDialect`, beside its codec in
//! `anthropic/`, `openai/` or `codex/`. Every module of `transport/` in the
//! crate's module tree, tests included, is held to an allowlist:
//! - its file lies in `transport/`: a `#[path]` or `include!` reaching
//!   outside it is refused, and every file there is a module of it;
//! - every crate path it names, resolved (`super::`/`self::` from the
//!   module's place in the tree; `use` leaves; paths in macro tokens),
//!   starts with an [`ALLOWED`] prefix — or, in a test module only, a
//!   [`TEST_RIG_ALLOWED`] one — and a crate or module alias, which cannot be
//!   followed, is refused;
//! - it invokes only the macros of [`MACROS`], so no `concat!`,
//!   `include_str!` or local `macro_rules!` assembles a name the scan cannot
//!   see, and imports or renames nothing to one of their names;
//! - no identifier, and no literal (decoded, so no escape hides a letter)
//!   outside a `#[doc = "…"]` attribute (what comments are), names a vendor
//!   ([`VENDOR_WORDS`]).
use super::{dependency_scan, swarm_board_ports::crate_paths};
use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};
use quote::ToTokens;
use std::path::{Path, PathBuf};
use syn::visit::Visit;

const TRANSPORT: &str = "src/infrastructure/providers/transport";

/// The module path of `transport/`.
const MODULE: [&str; 3] = ["infrastructure", "providers", "transport"];

/// The crate paths any module of `transport/` may name: the inner layers
/// and itself.
const ALLOWED: &[&str] = &[
    "crate::domain::",
    "crate::application::",
    "crate::infrastructure::providers::transport::",
];

/// The crate paths only `transport/`'s test modules may also name, for
/// their end-to-end rigs: the vendor-neutral retry decorator and the agent
/// loop's tool registry (#2605 review: never production's).
const TEST_RIG_ALLOWED: &[&str] = &[
    "crate::infrastructure::providers::routing::retry::",
    "crate::infrastructure::tools::registry::",
];

/// The macros `transport/` may invoke (by their last path segment).
const MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "debug_assert",
    "format",
    "join",
    "json",
    "matches",
    "panic",
    "pin",
    "select",
    "vec",
    "write",
];

/// The vendors, as they may appear in an identifier or a literal: any case,
/// any position (`AnthropicQuirk`, `openai_wire`, `"OpenAI"`, `b"codex"`).
const VENDOR_WORDS: &[&str] = &["openai", "anthropic", "codex", "claude"];

fn allowed(path: &str, test: bool) -> bool {
    let rig = test
        && TEST_RIG_ALLOWED
            .iter()
            .any(|prefix| path.starts_with(prefix));
    path != dependency_scan::UNRESOLVABLE
        && (rig || ALLOWED.iter().any(|prefix| path.starts_with(prefix)))
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

/// Whether `group`, after `before`, is a documentation attribute holding
/// only text: `#[doc = "…"]` or `#![doc = "…"]`, which comments are.
fn doc_text(before: &[TokenTree], group: &proc_macro2::Group) -> bool {
    let attribute = match before {
        [.., TokenTree::Punct(hash), TokenTree::Punct(bang)] => {
            hash.as_char() == '#' && bang.as_char() == '!'
        }
        [.., TokenTree::Punct(hash)] => hash.as_char() == '#',
        _ => false,
    };
    let inner: Vec<TokenTree> = group.stream().into_iter().collect();
    let text = matches!(
        inner.as_slice(),
        [TokenTree::Ident(doc), TokenTree::Punct(eq), TokenTree::Literal(_)]
            if doc == "doc" && eq.as_char() == '='
    );
    attribute && group.delimiter() == Delimiter::Bracket && text
}

/// Whether `ident` is a keyword (`if`, `return`, …), which a `!` after
/// negates rather than invokes: a keyword is no identifier syn accepts.
fn keyword(ident: &proc_macro2::Ident) -> bool {
    syn::parse2::<syn::Ident>(ident.to_token_stream()).is_err()
}

/// What `literal` says, decoded (escapes resolved, #2605 review round 3):
/// a string, byte string, C string, character or byte; otherwise its text.
fn decoded(literal: &proc_macro2::Literal) -> String {
    match syn::parse2::<syn::Lit>(literal.to_token_stream()) {
        Ok(syn::Lit::Str(text)) => text.value(),
        Ok(syn::Lit::ByteStr(bytes)) => String::from_utf8_lossy(&bytes.value()).into_owned(),
        Ok(syn::Lit::CStr(text)) => text.value().to_string_lossy().into_owned(),
        Ok(syn::Lit::Char(c)) => c.value().to_string(),
        Ok(syn::Lit::Byte(b)) => char::from(b.value()).to_string(),
        _ => literal.to_string(),
    }
}

/// What `stream` breaks of the token rules: each identifier and literal
/// naming a vendor outside documentation text, each macro invoked outside
/// [`MACROS`], and each rename to one of their names.
fn token_violations(stream: TokenStream, found: &mut Vec<String>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (at, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Group(group) => {
                if !doc_text(&tokens[..at], group) {
                    token_violations(group.stream(), found);
                }
            }
            TokenTree::Ident(ident) => {
                let word = ident.to_string().to_ascii_lowercase().replace('_', "");
                if VENDOR_WORDS.iter().any(|vendor| word.contains(vendor)) {
                    found.push(ident.to_string());
                }
                // A `!` alone after a name invokes it, whatever follows
                // (`macro_rules! m { … }` too); `!=` is joint.
                let invoked = matches!(
                    tokens.get(at + 1),
                    Some(TokenTree::Punct(bang))
                        if bang.as_char() == '!' && bang.spacing() == Spacing::Alone
                );
                if invoked && !keyword(ident) && !MACROS.contains(&ident.to_string().as_str()) {
                    found.push(format!("{ident}!"));
                }
                let renamed = matches!(
                    tokens.get(at + 1),
                    Some(TokenTree::Ident(to)) if ident == "as" && MACROS.contains(&to.to_string().as_str())
                );
                if renamed {
                    found.push(format!("as {}", tokens[at + 1]));
                }
            }
            TokenTree::Literal(literal) => {
                let text = decoded(literal).to_ascii_lowercase();
                if VENDOR_WORDS.iter().any(|vendor| text.contains(vendor)) {
                    found.push(literal.to_string());
                }
            }
            TokenTree::Punct(_) => {}
        }
    }
}

/// The `use` leaves of a file named like an allowed macro: an import that
/// would let another macro be invoked by that name (#2605 review round 3).
#[derive(Default)]
struct MacroImports(Vec<String>);

impl<'ast> Visit<'ast> for MacroImports {
    fn visit_use_name(&mut self, name: &'ast syn::UseName) {
        if MACROS.contains(&name.ident.to_string().as_str()) {
            self.0.push(format!("use …::{}", name.ident));
        }
    }
    fn visit_use_rename(&mut self, rename: &'ast syn::UseRename) {
        for ident in [&rename.ident, &rename.rename] {
            if MACROS.contains(&ident.to_string().as_str()) {
                self.0
                    .push(format!("use …::{} as {}", rename.ident, rename.rename));
            }
        }
    }
}

/// What `source`, the module `module` (a test module when `test`), breaks:
/// each crate path outside its allowlist and each token rule.
fn violations(source: &str, module: &[String], test: bool) -> Vec<String> {
    let mut found: Vec<String> = crate_paths(source, module)
        .into_iter()
        .filter(|path| !allowed(path, test))
        .collect();
    let file = syn::parse_file(source).expect("a crate source parses");
    token_violations(file.to_token_stream(), &mut found);
    let mut imports = MacroImports::default();
    imports.visit_file(&file);
    found.extend(imports.0);
    found
}

/// The modules mounted under `transport/` (`mounted`) whose file lies
/// outside `folder` — reached by a `#[path]`, a `#[cfg_attr(…, path)]`, an
/// `include!` or a test fixture's mount — and the files in `folder` (`in_folder`)
/// that are no such module.
fn escapes(
    mounted: &[(PathBuf, Vec<String>, bool)],
    folder: &Path,
    in_folder: &[PathBuf],
) -> Vec<String> {
    let outside = mounted
        .iter()
        .filter(|(file, _, _)| !file.starts_with(folder))
        .map(|(file, _, _)| format!("outside: {}", file.display()));
    let unmounted = in_folder
        .iter()
        .filter(|file| !mounted.iter().any(|(mounted, _, _)| mounted == *file))
        .map(|file| format!("unmounted: {}", file.display()));
    let mut found: Vec<String> = outside.chain(unmounted).collect();
    found.sort();
    found.dedup();
    found
}

#[test]
fn transport_names_only_vendor_neutral_paths_and_no_vendor() {
    let mounted = dependency_scan::mounted_under(&MODULE);
    let mut files = Vec::new();
    rust_files(Path::new(TRANSPORT), &mut files);
    let escaped = escapes(&mounted, Path::new(TRANSPORT), &files);
    assert!(
        escaped.is_empty(),
        "transport/'s modules and its files must be one set: {escaped:?}"
    );
    let names: Vec<String> = mounted
        .iter()
        .filter_map(|(file, _, _)| file.file_name()?.to_str().map(str::to_owned))
        .collect();
    assert!(
        [
            "attempt_observer.rs",
            "attempt_profile.rs",
            "wire_dialect.rs"
        ]
        .iter()
        .all(|name| names.iter().any(|found| found == name))
            && mounted.iter().any(|(_, _, test)| *test),
        "the scan reads transport's production modules and its tests: {names:?}"
    );
    let found: Vec<String> = mounted
        .iter()
        .flat_map(|(file, module, test)| {
            let source = std::fs::read_to_string(file).expect("read a source");
            violations(&source, module, *test)
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
        // #2605 review round 2.
        "const BYTES: &[u8] = b\"anthropic\";",
        "const RAW: &[u8] = br\"OpenAI\";",
        "fn l(doc: u8) -> [u8; 2] { [doc, b'x'] } const M: [&str; 1] = [\"openai\"];",
        "fn n(doc: &str) -> [&str; 2] { [doc, \"openai\"] }",
        "const JOINED: &str = concat!(\"anth\", \"ropic\");",
        "const FILE: &str = include_str!(concat!(\"../\", \"x.rs\"));",
        "#[doc = include_str!(\"vendor.md\")]\nfn o() {}",
        "use crate::infrastructure::providers::ProviderBindingWire;",
        "use crate::infrastructure::providers::AttemptTransportBinding;",
        // #2605 review round 3.
        "macro_rules! join { ($($s:tt)*) => { $($s)::* } }\n\
         fn p(_: join!(crate infrastructure providers routing retry RetryConfig)) {}",
        "use std::concat as pin;\nconst P: &str = pin!(\"a\", \"b\");",
        "use std::include_str as vec;",
        "fn q() { let _ = { use std::concat as select; 1 }; }",
        "const ESCAPED: &str = \"\\x6fpenai\";",
        "const UNICODE: &str = \"c\\u{6f}dex\";",
        "const BYTES_ESCAPED: &[u8] = b\"\\x63laude\";",
        "const CHARS: [char; 2] = ['a', '\\x41']; const C: &str = \"\\u{41}nthropic\";",
    ] {
        for test in [false, true] {
            assert!(
                !violations(evasion, &here, test).is_empty(),
                "not caught (test module: {test}): {evasion}"
            );
        }
    }
    // A test rig's reach is a test module's only.
    for rig in [
        "use crate::infrastructure::providers::routing::retry::RetryConfig;",
        "fn p() { let _ = crate::infrastructure::tools::registry::ToolRegistryImpl::new(); }",
        "use super::super::routing::retry::SHARED_WIRE;",
    ] {
        assert!(
            !violations(rig, &here, false).is_empty(),
            "production may not reach a test rig: {rig}"
        );
        assert!(
            violations(rig, &here, true).is_empty(),
            "a test module may: {rig}: {:?}",
            violations(rig, &here, true)
        );
    }
    for neutral in [
        "use crate::domain::shared::error::DomainError;",
        "use super::sse_common::event_data;",
        "use crate::infrastructure::providers::transport::stream_idle::StreamIdle;",
        "/// Codex and OpenAI streams alike.\nfn j() {}",
        "fn k(vendor: &str) -> bool { vendor.is_empty() }",
        "const DONE: &str = \"[DONE]\";",
        "#![doc = \"Anthropic, inner.\"]\nfn q() { assert!(matches!(1, 1), \"{}\", format!(\"x\")); }",
        "#[doc = \"OpenAI\"]\nfn r() { debug_assert!(vec![1u8].len() == 1); }",
        "use std::pin::Pin;\nfn s(a: u8, b: u8) -> bool { a != b && !(a == 0) }",
        "fn t(x: bool) -> bool { if !x { return !x; } while !x {} match !x { _ => !x } }",
        "const PLAIN: &str = \"\\x41\\u{42}\";",
    ] {
        for test in [false, true] {
            assert!(
                violations(neutral, &here, test).is_empty(),
                "refused: {neutral}: {:?}",
                violations(neutral, &here, test)
            );
        }
    }
}

/// #2605 review round 3: a module of `transport/` whose file lies outside
/// it — by `#[path]`, `#[cfg_attr(…, path)]`, `include!` or a test fixture's
/// mount of a production file, whichever is mounted first — and a file there
/// that is no module of it are each refused; a folder whose files are its
/// modules is not.
#[test]
fn every_way_out_of_the_folder_is_refused() {
    let transport = "mod inner;\n\
        #[path = \"../other/away.rs\"] mod away;\n\
        include!(\"../spliced.rs\");\n\
        #[cfg(test)] #[path = \"../other/prod.rs\"] mod fixture;\n\
        #[cfg_attr(all(), path = \"../other/attr.rs\")] mod conditional;";
    for lib in ["mod transport;\nmod other;", "mod other;\nmod transport;"] {
        let files = [
            ("lib.rs", lib),
            ("transport/mod.rs", transport),
            ("transport/inner.rs", ""),
            ("transport/stray.rs", ""),
            ("other/mod.rs", "mod prod;"),
            ("other/prod.rs", ""),
            ("other/away.rs", ""),
            ("other/attr.rs", ""),
            ("spliced.rs", ""),
        ];
        let in_folder: Vec<PathBuf> = files
            .iter()
            .map(|(path, _)| PathBuf::from(path))
            .filter(|path| path.starts_with("transport"))
            .collect();
        let mounted = dependency_scan::fixture_mounted_under(&files, &["transport"]);
        assert_eq!(
            escapes(&mounted, Path::new("transport"), &in_folder),
            [
                "outside: other/attr.rs",
                "outside: other/away.rs",
                "outside: other/prod.rs",
                "outside: spliced.rs",
                "unmounted: transport/stray.rs",
            ],
            "{lib}"
        );
    }
    let files = [
        ("lib.rs", "mod transport;"),
        (
            "transport/mod.rs",
            "mod inner;\n#[cfg(test)] #[path = \"inner_tests.rs\"] mod tests;",
        ),
        ("transport/inner.rs", ""),
        ("transport/inner_tests.rs", ""),
    ];
    let in_folder: Vec<PathBuf> = files[1..]
        .iter()
        .map(|(path, _)| PathBuf::from(path))
        .collect();
    let mounted = dependency_scan::fixture_mounted_under(&files, &["transport"]);
    assert!(escapes(&mounted, Path::new("transport"), &in_folder).is_empty());
}
