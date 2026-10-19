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
//! - it invokes only the std prelude macros of [`PRELUDE_MACROS`] by name
//!   and the others of [`PATHED_MACROS`] by their full path, so no
//!   `concat!`, `include_str!` or local `macro_rules!` assembles a name the
//!   scan cannot see; it imports or renames nothing to one of their names,
//!   and no module of the crate defines or re-exports one, nor does an
//!   ancestor of `transport/` `#[macro_use]` anything;
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

/// The std prelude macros `transport/` may invoke by their bare name.
const PRELUDE_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "debug_assert",
    "format",
    "matches",
    "panic",
    "vec",
    "write",
];

/// The other macros `transport/` may invoke, each only by its full path
/// (#2605 review round 4): never bare, nor through `crate::`, `super::` or
/// `self::`, where a macro of the same name could be defined.
const PATHED_MACROS: &[&str] = &[
    "serde_json::json",
    "tokio::join",
    "tokio::pin",
    "tokio::select",
];

/// Every name an allowed macro is invoked by: what no `use` in `transport/`
/// may import or rename to, and no module of the crate may define or
/// re-export.
fn macro_names() -> impl Iterator<Item = &'static str> {
    PRELUDE_MACROS.iter().copied().chain(
        PATHED_MACROS
            .iter()
            .filter_map(|path| path.rsplit("::").next()),
    )
}

fn macro_name(name: &str) -> bool {
    macro_names().any(|allowed| allowed == name)
}

/// The path a macro named at `tokens[at]` is invoked by: its name and the
/// `a::b::` segments before it.
fn invoked_path(tokens: &[TokenTree], at: usize) -> String {
    let mut segments = vec![tokens[at].to_string()];
    let mut from = at;
    while from >= 3 {
        let colons = matches!(
            (&tokens[from - 2], &tokens[from - 1]),
            (TokenTree::Punct(a), TokenTree::Punct(b)) if a.as_char() == ':' && b.as_char() == ':'
        );
        match (colons, &tokens[from - 3]) {
            (true, TokenTree::Ident(segment)) => segments.insert(0, segment.to_string()),
            _ => break,
        }
        from -= 3;
    }
    // A leading `::` names no allowed path.
    let rooted = from >= 2
        && matches!(
            (&tokens[from - 2], &tokens[from - 1]),
            (TokenTree::Punct(a), TokenTree::Punct(b)) if a.as_char() == ':' && b.as_char() == ':'
        );
    match rooted {
        true => format!("::{}", segments.join("::")),
        false => segments.join("::"),
    }
}

/// Whether the macro invoked by `path` is allowed: a prelude macro by its
/// bare name, or another by its full path.
fn allowed_macro(path: &str) -> bool {
    PRELUDE_MACROS.contains(&path) || PATHED_MACROS.contains(&path)
}

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
/// naming a vendor outside documentation text, each macro invoked other
/// than as [`PRELUDE_MACROS`] or [`PATHED_MACROS`] allow, and each rename to
/// one of their names.
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
                if invoked && !keyword(ident) {
                    let path = invoked_path(&tokens, at);
                    if !allowed_macro(&path) {
                        found.push(format!("{path}!"));
                    }
                }
                let renamed = matches!(
                    tokens.get(at + 1),
                    Some(TokenTree::Ident(to)) if ident == "as" && macro_name(&to.to_string())
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
        if macro_name(&name.ident.to_string()) {
            self.0.push(format!("use …::{}", name.ident));
        }
    }
    fn visit_use_rename(&mut self, rename: &'ast syn::UseRename) {
        for ident in [&rename.ident, &rename.rename] {
            if macro_name(&ident.to_string()) {
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
    // Paths an attribute names in a string (`deserialize_with = "…"`).
    found.extend(
        dependency_scan::string_paths_in_tokens(file.to_token_stream())
            .into_iter()
            .map(|path| match path.as_str() {
                dependency_scan::UNRESOLVABLE => path,
                _ => dependency_scan::resolve_relative(module, &path),
            })
            .filter(|path| !allowed(path, test)),
    );
    token_violations(file.to_token_stream(), &mut found);
    let mut imports = MacroImports::default();
    imports.visit_file(&file);
    found.extend(imports.0);
    found
}

/// The macro definitions and re-exports of `source` under an allowed
/// macro's name, and its `#[macro_use]` attributes (#2605 review round 4):
/// what could stand in for an allowed macro in `transport/`.
fn macro_stand_ins(source: &str) -> Vec<String> {
    #[derive(Default)]
    struct StandIns(Vec<String>);
    impl<'ast> Visit<'ast> for StandIns {
        fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
            let defined = item.mac.path.is_ident("macro_rules")
                && item
                    .ident
                    .as_ref()
                    .is_some_and(|name| macro_name(&name.to_string()));
            if defined {
                self.0
                    .push(format!("macro_rules! {}", item.ident.as_ref().unwrap()));
            }
            syn::visit::visit_item_macro(self, item);
        }
        fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
            if !matches!(item.vis, syn::Visibility::Inherited) {
                let mut leaves = MacroImports::default();
                leaves.visit_item_use(item);
                self.0
                    .extend(leaves.0.into_iter().map(|leaf| format!("pub {leaf}")));
            }
            syn::visit::visit_item_use(self, item);
        }
        fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
            if attr.path().is_ident("macro_use") {
                self.0.push("#[macro_use]".to_string());
            }
        }
    }
    let file = syn::parse_file(source).expect("a crate source parses");
    let mut found = StandIns::default();
    found.visit_file(&file);
    found.0
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

/// #2605 review round 4: no module of either crate defines or re-exports a
/// macro under an allowed macro's name, nor `#[macro_use]`s one, so an
/// allowed name in `transport/` — bare, through a glob import, or from an
/// ancestor's textual scope — is the macro the allowlist means.
#[test]
fn no_module_stands_in_for_an_allowed_macro() {
    let modules = dependency_scan::mounted_under(&[]);
    assert!(modules.len() > 500, "both crates were walked");
    let found: Vec<String> = modules
        .iter()
        .flat_map(|(file, _, _)| {
            let source = std::fs::read_to_string(file).expect("read a source");
            macro_stand_ins(&source)
                .into_iter()
                .map(move |hit| format!("{}: {hit}", file.display()))
        })
        .collect();
    assert!(
        found.is_empty(),
        "a stand-in for an allowed macro: {found:#?}"
    );
    for stand_in in [
        "macro_rules! select { () => {} }\npub mod transport {}",
        "#[macro_export] macro_rules! vec { () => {} }",
        "pub use std::concat as vec;",
        "pub(crate) use tokio::select;",
        "pub(in crate::domain) use std::{concat as json};",
        "#[macro_use] mod helpers;",
    ] {
        assert!(
            !macro_stand_ins(stand_in).is_empty(),
            "not caught: {stand_in}"
        );
    }
    for neutral in [
        "use serde_json::json;",
        "macro_rules! bind_attempt_admission { () => {} }",
        "pub use std::pin::Pin;",
    ] {
        assert!(macro_stand_ins(neutral).is_empty(), "refused: {neutral}");
    }
}

/// #2605 review round 4 (A3): each rename guard catches on its own what the
/// other cannot see — an import outside macro tokens, and a rename inside
/// them.
#[test]
fn each_rename_guard_catches_its_own_case() {
    let imported = "use crate::domain::x::vec;";
    let file = syn::parse_file(imported).unwrap();
    let mut imports = MacroImports::default();
    imports.visit_file(&file);
    assert!(!imports.0.is_empty(), "the import visitor: {imported}");
    let mut tokens = Vec::new();
    token_violations(file.to_token_stream(), &mut tokens);
    assert!(tokens.is_empty(), "not the token rule: {tokens:?}");
    let inside = "fn f() { tokio::select! { _ = async { use std::concat as pin; } => {} } }";
    let file = syn::parse_file(inside).unwrap();
    let mut imports = MacroImports::default();
    imports.visit_file(&file);
    assert!(
        imports.0.is_empty(),
        "not the import visitor: {:?}",
        imports.0
    );
    let mut tokens = Vec::new();
    token_violations(file.to_token_stream(), &mut tokens);
    assert_eq!(tokens, ["as pin"], "the token rule: {inside}");
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
        // #2605 review round 4.
        "fn a() { crate::domain::shared::vec!(); }",
        "fn b() { super::select!(); }",
        "fn c() { self::json!(); }",
        "use crate::domain::shared::*;\nfn d() { select! { _ = async {} => {} } }",
        "fn e() { let _ = json!({}); }",
        "fn g() { ::tokio::join!(); }",
        "#[derive(serde::Deserialize)]\nstruct S { #[serde(deserialize_with = \"crate::infrastructure::providers::routing::provider_factory::f\")] f: u8 }",
        "#[derive(serde::Deserialize)]\nstruct T { #[serde(deserialize_with = \"super::super::routing::router::f\")] f: u8 }",
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
        "fn h(x: u8) { tokio::select! { _ = async {} => {} } let _ = serde_json::json!({}); tokio::pin!(x); }",
        "#[derive(serde::Deserialize)]\nstruct N { #[serde(deserialize_with = \"crate::domain::x::f\")] f: u8 }",
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

/// The error names the wires declare, read from the source of every module
/// of `providers/` outside `transport/` that implements `WireDialect`
/// (#2620 review). `ErrorName`'s fields are private, so its `const`
/// constructors (`ErrorName::throttle("…", …)` and its siblings) are the
/// only way to declare one: each must name a string literal, and each
/// file's names must number exactly its `[ErrorName; N]` tables.
fn wire_error_names() -> Vec<String> {
    wire_sources()
        .iter()
        .flat_map(|(file, source, _)| declared_error_names(source, &file.display().to_string()))
        .collect()
}

/// The error names `source` (the file `file`) declares; panics on an
/// `ErrorName` named by anything but a string literal, or names that do not
/// number its `[ErrorName; N]` tables.
fn declared_error_names(source: &str, file: &str) -> Vec<String> {
    #[derive(Default)]
    struct Names {
        names: Vec<String>,
        declared: usize,
    }
    impl<'ast> Visit<'ast> for Names {
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            let segments: Vec<String> = match &*call.func {
                syn::Expr::Path(path) => path
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect(),
                _ => Vec::new(),
            };
            let constructor = segments.len() >= 2 && segments[segments.len() - 2] == "ErrorName";
            if constructor {
                match call.args.first() {
                    Some(syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(name),
                        ..
                    })) => self.names.push(name.value()),
                    other => panic!(
                        "an ErrorName must name a string literal: {}",
                        other.map_or(String::new(), |arg| arg.to_token_stream().to_string())
                    ),
                }
            }
            syn::visit::visit_expr_call(self, call);
        }
        fn visit_type_array(&mut self, array: &'ast syn::TypeArray) {
            let of_names = matches!(
                &*array.elem,
                syn::Type::Path(path) if path.path.segments.last().is_some_and(|last| last.ident == "ErrorName")
            );
            if of_names {
                match &array.len {
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Int(count),
                        ..
                    }) => self.declared += count.base10_parse::<usize>().expect("a table length"),
                    other => panic!(
                        "an ErrorName table has a literal length: {}",
                        other.to_token_stream()
                    ),
                }
            }
            syn::visit::visit_type_array(self, array);
        }
    }
    let mut names = Names::default();
    names.visit_file(&syn::parse_file(source).expect("a wire parses"));
    assert_eq!(
        names.names.len(),
        names.declared,
        "{file}: the error names declared are those its tables hold"
    );
    names.names
}

/// #2620 review: an error name declared any other way than a constructor
/// on a string literal, or a table the names do not fill, is refused.
#[test]
fn error_names_are_declared_only_as_literals_in_counted_tables() {
    let declared = "static E: [ErrorName; 2] = [\
        ErrorName::throttle(\"a\", ErrorCode::Overloaded),\
        crate::x::ErrorName::plain(\"b\", ErrorCode::ServerError)];";
    assert_eq!(declared_error_names(declared, "fixture"), ["a", "b"]);
    for refused in [
        "const N: &str = \"overloaded_error\";\nstatic E: [ErrorName; 1] = [ErrorName::throttle(N, ErrorCode::Overloaded)];",
        "static E: [ErrorName; 2] = [ErrorName::throttle(\"a\", ErrorCode::Overloaded)];",
        "static E: [ErrorName; 1] = [ErrorName::plain(concat!(\"a\", \"b\"), ErrorCode::ServerError)];",
    ] {
        let caught = std::panic::catch_unwind(|| declared_error_names(refused, "fixture"));
        assert!(caught.is_err(), "not refused: {refused}");
    }
}

/// Every literal of `stream`, decoded.
fn literals(stream: TokenStream, found: &mut Vec<String>) {
    for token in stream {
        match token {
            TokenTree::Group(group) => literals(group.stream(), found),
            TokenTree::Literal(literal) => found.push(decoded(&literal)),
            TokenTree::Ident(_) | TokenTree::Punct(_) => {}
        }
    }
}

/// #2620 review: a wire's error vocabulary lives beside its dialect. No
/// production module of `transport/` spells any wire's error name, so none
/// slides back into the shared readers.
#[test]
fn transport_spells_no_wire_error_name() {
    let names = wire_error_names();
    for expected in ["rate_limit_exceeded", "overloaded_error", "invalid_api_key"] {
        assert!(
            names.iter().any(|name| name == expected),
            "{expected} in {names:?}"
        );
    }
    let found: Vec<String> = dependency_scan::mounted_under(&MODULE)
        .iter()
        .filter(|(_, _, test)| !test)
        .flat_map(|(file, _, _)| {
            let source = std::fs::read_to_string(file).expect("read a source");
            let mut spelled = Vec::new();
            literals(
                syn::parse_file(&source)
                    .expect("a source parses")
                    .to_token_stream(),
                &mut spelled,
            );
            spelled
                .into_iter()
                .filter(|literal| names.contains(literal))
                .map(move |literal| format!("{}: {literal:?}", file.display()))
        })
        .collect();
    assert!(
        found.is_empty(),
        "a wire's error name in transport/: {found:#?}"
    );
}

/// The event names the wires declare: every string in each wire's
/// `Vocabulary` statics (its known events and its terminal ones), read from
/// the source of every module of `providers/` outside `transport/` that
/// implements `WireDialect` (#2609 review round 2).
fn wire_event_names() -> Vec<String> {
    wire_sources()
        .iter()
        .flat_map(|(_, _, parsed)| vocabulary_names(parsed))
        .collect()
}

/// Whether `file` implements `WireDialect` anywhere: an `impl` whose trait
/// path ends in `WireDialect`, however it is named (#2620 review round 3).
fn implements_wire_dialect(file: &syn::File) -> bool {
    #[derive(Default)]
    struct Found(bool);
    impl<'ast> Visit<'ast> for Found {
        fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
            let dialect = item.trait_.as_ref().is_some_and(|(_, path, _)| {
                path.segments
                    .last()
                    .is_some_and(|last| last.ident == "WireDialect")
            });
            self.0 |= dialect;
            syn::visit::visit_item_impl(self, item);
        }
    }
    let mut found = Found::default();
    found.visit_file(file);
    found.0
}

/// Every module of `providers/` outside `transport/` that implements
/// `WireDialect`: its path, source and parse. At least the three wires.
fn wire_sources() -> Vec<(PathBuf, String, syn::File)> {
    let mut files = Vec::new();
    rust_files(Path::new("src/infrastructure/providers"), &mut files);
    let wires: Vec<(PathBuf, String, syn::File)> = files
        .into_iter()
        .filter(|file| !file.starts_with(TRANSPORT))
        .filter_map(|file| {
            let source = std::fs::read_to_string(&file).expect("read a source");
            let parsed = syn::parse_file(&source).expect("a source parses");
            implements_wire_dialect(&parsed).then_some((file, source, parsed))
        })
        .collect();
    assert!(wires.len() >= 3, "every wire was read ({})", wires.len());
    wires
}

/// #2620 review round 3: a wire is found by its `impl`, however its trait
/// is named, and its vocabulary is read from a `const` as from a `static`.
#[test]
fn a_wire_is_found_by_its_impl_and_read_from_any_item() {
    for named in [
        "impl WireDialect for X {}",
        "impl wire_dialect::WireDialect for X {}",
        "mod inner { impl crate::infrastructure::providers::transport::wire_dialect::WireDialect for X {} }",
    ] {
        assert!(
            implements_wire_dialect(&syn::parse_file(named).unwrap()),
            "{named}"
        );
    }
    for unrelated in [
        "impl Other for X {}",
        "impl X {}",
        "// impl WireDialect for X\nfn f() {}",
    ] {
        assert!(
            !implements_wire_dialect(&syn::parse_file(unrelated).unwrap()),
            "{unrelated}"
        );
    }
    let constant = "const V: Vocabulary = Vocabulary { known: &[\"a.b\"], terminal: &[] };\n\
        static W: transport::Vocabulary = Vocabulary { known: &[\"c\"], terminal: &[] };";
    assert_eq!(
        vocabulary_names(&syn::parse_file(constant).unwrap()),
        ["a.b", "c"]
    );
}

/// The strings of `file`'s `Vocabulary` items, `static` or `const`.
fn vocabulary_names(file: &syn::File) -> Vec<String> {
    #[derive(Default)]
    struct Names(Vec<String>);
    fn vocabulary(ty: &syn::Type) -> bool {
        matches!(ty, syn::Type::Path(path) if path.path.segments.last().is_some_and(|last| last.ident == "Vocabulary"))
    }
    impl<'ast> Visit<'ast> for Names {
        fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
            if vocabulary(&item.ty) {
                literals(item.expr.to_token_stream(), &mut self.0);
            }
            syn::visit::visit_item_static(self, item);
        }
        fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
            if vocabulary(&item.ty) {
                literals(item.expr.to_token_stream(), &mut self.0);
            }
            syn::visit::visit_item_const(self, item);
        }
    }
    let mut names = Names::default();
    names.visit_file(file);
    names.0
}

/// #2609 review round 2: a wire's events live beside its dialect. No
/// production item of `transport/` spells any wire's event name, so none
/// slides back into the shared mechanics.
#[test]
fn transport_spells_no_wire_event_name() {
    let names = wire_event_names();
    for expected in [
        "response.completed",
        "response.incomplete",
        "message_stop",
        "ping",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "{expected} in {names:?}"
        );
    }
    let found: Vec<String> = dependency_scan::mounted_under(&MODULE)
        .iter()
        .filter(|(_, _, test)| !test)
        .flat_map(|(file, _, _)| {
            let source = std::fs::read_to_string(file).expect("read a source");
            // Its production items only: a `#[cfg(test)]` item (the test
            // wire) is the tests'.
            let mut spelled = Vec::new();
            for item in syn::parse_file(&source).expect("a source parses").items {
                if !dependency_scan::item_test_only(&item) {
                    literals(item.to_token_stream(), &mut spelled);
                }
            }
            spelled
                .into_iter()
                .filter(|literal| names.contains(literal))
                .map(move |literal| format!("{}: {literal:?}", file.display()))
        })
        .collect();
    assert!(
        found.is_empty(),
        "a wire's event name in transport/: {found:#?}"
    );
}

/// Every string literal of `stream` (byte and C strings too) outside
/// documentation text, decoded: numbers name no wire.
fn code_literals(stream: TokenStream, found: &mut Vec<String>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (at, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Group(group) if !doc_text(&tokens[..at], group) => {
                code_literals(group.stream(), found)
            }
            TokenTree::Literal(literal) => {
                let string = matches!(
                    syn::parse2::<syn::Lit>(literal.to_token_stream()),
                    Ok(syn::Lit::Str(_) | syn::Lit::ByteStr(_) | syn::Lit::CStr(_))
                );
                if string {
                    found.push(decoded(literal));
                }
            }
            _ => {}
        }
    }
}

/// The literals `transport/` shares with the wires' dialects, each because
/// it names nothing of one wire: the empty string, the `type` field every
/// typed payload carries (bare, and quoted as a long event's start spells
/// it), and the `response` a failed read's message names.
const SHARED_LITERALS: &[&str] = &["", "type", "\"type\"", "response"];

/// #2609 review round 2 (completed here, once the error vocabulary has
/// moved): no production module of `transport/` spells a literal any
/// wire's dialect spells — an event name, an error name, a field or reason
/// only that wire sends (`incomplete_details`, `max_output_tokens`) — but
/// for [`SHARED_LITERALS`].
#[test]
fn transport_shares_no_literal_with_a_wire_but_the_neutral_ones() {
    let mut spelled_by_wires = Vec::new();
    for (_, _, parsed) in wire_sources() {
        code_literals(parsed.to_token_stream(), &mut spelled_by_wires);
    }
    for expected in [
        "incomplete_details",
        "max_output_tokens",
        "response.completed",
        "overloaded_error",
    ] {
        assert!(
            spelled_by_wires.iter().any(|literal| literal == expected),
            "{expected} is a wire's"
        );
    }
    let found: Vec<String> = dependency_scan::mounted_under(&MODULE)
        .iter()
        .filter(|(_, _, test)| !test)
        .flat_map(|(file, _, _)| {
            let source = std::fs::read_to_string(file).expect("read a source");
            // Its production items only: a `#[cfg(test)]` item (the test
            // wire) is the tests'.
            let mut spelled = Vec::new();
            for item in syn::parse_file(&source).expect("a source parses").items {
                if !dependency_scan::item_test_only(&item) {
                    code_literals(item.to_token_stream(), &mut spelled);
                }
            }
            spelled
                .into_iter()
                .filter(|literal| {
                    spelled_by_wires.contains(literal)
                        && !SHARED_LITERALS.contains(&literal.as_str())
                })
                .map(move |literal| format!("{}: {literal:?}", file.display()))
        })
        .collect();
    assert!(
        found.is_empty(),
        "a wire's literal in transport/: {found:#?}"
    );
}
