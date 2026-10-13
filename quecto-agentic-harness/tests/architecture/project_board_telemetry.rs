//! A `board_op` record holds no text, and is written only while the event
//! log is on (epic #2481 T1). The threat is accidental leakage by a later
//! change, so each guard makes such a change fail here, where a reviewer
//! sees it:
//!
//! - the record's module, its leaves' module and the commit id's module
//!   each match a whole-file snapshot (`snapshots/*.tokens`, doc comments
//!   left out), so any change to them, bodies and macros included, is a
//!   reviewed change to a snapshot;
//! - every field reachable from `BoardOpRecord` is a kind, a count, a
//!   guarded leaf or a record of those;
//! - the op-log port, the recording switch, `BoardRecording::when` and
//!   T2's adapters are named only where [`NAMED_ONLY_IN`] allows, never
//!   aliased, and never in a public signature in composition;
//! - the board's modules, its CLI and its composition write no `tracing`
//!   record but through the adapters' file, whose adapters a recording
//!   holds;
//! - every op of the board's `OPS` table has its `OpKind`, and every name
//!   a record holds today is a known kind.
use std::collections::BTreeMap;
use std::path::Path;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit::Visit;

use quecto::domain::project_board::value_objects::telemetry::{OpKind, StaticKind};
use quecto::domain::project_board::value_objects::vocabulary::TaskStatus;
use quecto::interface::tools::project_board::request::OPS;

use super::dependency_scan;

const RECORD_FILE: &str = "src/domain/project_board/value_objects/telemetry/mod.rs";

/// Each guarded file and its snapshot.
const SNAPSHOTS: &[(&str, &str)] = &[
    (
        RECORD_FILE,
        "tests/architecture/snapshots/telemetry_mod.tokens",
    ),
    (
        "src/domain/project_board/value_objects/telemetry/leaves.rs",
        "tests/architecture/snapshots/telemetry_leaves.tokens",
    ),
    (
        "src/domain/project_board/value_objects/commit_id.rs",
        "tests/architecture/snapshots/commit_id.tokens",
    ),
];

/// Types a record field may hold as they are: counts, signed spans (T4's
/// claim seconds), flags (T3's `cancelled`), and the guarded leaves (the
/// commit id is `code_ref`'s, from `commit_id.rs`).
const ADMITTED: &[&str] = &["u64", "i64", "bool", "StaticKind", "TaskRef", "CommitId"];

/// The containers a field's types are read through: whatever they hold
/// (a `BTreeMap`'s keys and values alike) must be admitted in turn.
const CONTAINERS: &[&str] = &["Option", "Vec", "BTreeMap"];

/// The adapters' file (T2), the one place besides composition that may
/// name them.
const ADAPTER: &str = "src/infrastructure/persistence/project_board/op_log.rs";

/// Each name, and the only production paths (a file, or a directory
/// ending in `/`) that may name it.
const NAMED_ONLY_IN: &[(&str, &[&str])] = &[
    (
        "ProjectBoardOpLog",
        &[
            "src/application/project_board/ports.rs",
            "src/application/project_board/telemetry.rs",
            ADAPTER,
            "src/composition/",
        ],
    ),
    (
        "RecordingSwitch",
        &[
            "src/application/project_board/telemetry.rs",
            "src/composition/",
        ],
    ),
    (
        "BoardRecording::when",
        &[
            "src/application/project_board/telemetry.rs",
            "src/composition/",
        ],
    ),
    (
        "RecordingSwitch::resolved",
        &[
            "src/application/project_board/telemetry.rs",
            "src/composition/bootstrap/project_board.rs",
        ],
    ),
    ("EventLogProjectBoardOps", &[ADAPTER, "src/composition/"]),
    ("TracingProjectBoardOps", &[ADAPTER, "src/composition/"]),
];

/// The names no production code may alias (`use … as`, `type`).
const NEVER_ALIASED: &[&str] = &[
    "ProjectBoardOpLog",
    "RecordingSwitch",
    "BoardRecording",
    "EventLogProjectBoardOps",
    "TracingProjectBoardOps",
];

fn tokens(item: &impl ToTokens) -> String {
    item.to_token_stream().to_string()
}

/// `stream` without doc comments (`#[doc = …]`, `#![doc = …]`).
fn undocumented(stream: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let doc = |at: usize| match tokens.get(at) {
        Some(TokenTree::Group(group)) => {
            matches!(group.stream().into_iter().next(), Some(TokenTree::Ident(ident)) if ident == "doc")
        }
        _ => false,
    };
    let mut kept = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        let hash = matches!(&tokens[at], TokenTree::Punct(p) if p.as_char() == '#');
        let bang = matches!(tokens.get(at + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!');
        let skip = match (hash, bang) {
            (true, true) if doc(at + 2) => 3,
            (true, false) if doc(at + 1) => 2,
            _ => 0,
        };
        if skip > 0 {
            at += skip;
            continue;
        }
        kept.push(match &tokens[at] {
            TokenTree::Group(group) => {
                let mut inner =
                    proc_macro2::Group::new(group.delimiter(), undocumented(group.stream()));
                inner.set_span(group.span());
                TokenTree::Group(inner)
            }
            other => other.clone(),
        });
        at += 1;
    }
    kept.into_iter().collect()
}

/// `stream` as text, a space between tokens (none after a joint
/// punctuation mark); inside an array or a macro's group (not an
/// attribute's), and every group nested in one, each comma-separated
/// element on a line of its own, so a kind added or dropped is a line of
/// the snapshot's diff.
fn text(stream: TokenStream, out: &mut String, split: bool) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let punct = |at: Option<usize>, char: char| {
        at.and_then(|at| tokens.get(at))
            .is_some_and(|token| matches!(token, TokenTree::Punct(p) if p.as_char() == char))
    };
    for (at, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Group(group) => {
                let before = at.checked_sub(1);
                let attribute =
                    punct(before, '#') || (punct(before, '!') && punct(at.checked_sub(2), '#'));
                let listed = group.delimiter() == Delimiter::Bracket || punct(before, '!');
                let inner = (listed || split) && !attribute;
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ("(", ")"),
                    Delimiter::Brace => ("{", "}"),
                    Delimiter::Bracket => ("[", "]"),
                    Delimiter::None => ("", ""),
                };
                let gap = if inner { '\n' } else { ' ' };
                out.push_str(open);
                out.push(gap);
                text(group.stream(), out, inner);
                out.push(gap);
                out.push_str(close);
                out.push(' ');
            }
            TokenTree::Punct(p) if split && p.as_char() == ',' => out.push_str(",\n"),
            TokenTree::Punct(p) => {
                out.push(p.as_char());
                if p.spacing() == proc_macro2::Spacing::Alone {
                    out.push(' ');
                }
            }
            other => {
                out.push_str(&other.to_string());
                out.push(' ');
            }
        }
    }
}

/// `source` as its snapshot holds it: doc comments left out, each item
/// starting a line, arrays and macro groups one element a line.
fn rendered(source: &str) -> String {
    let file = syn::parse_file(source).expect("parses");
    let mut out = String::new();
    for attr in file
        .attrs
        .iter()
        .filter(|attr| !attr.path().is_ident("doc"))
    {
        text(attr.to_token_stream(), &mut out, false);
        out.push('\n');
    }
    for item in &file.items {
        text(undocumented(item.to_token_stream()), &mut out, false);
        out.push('\n');
    }
    let lines = out.lines().map(str::trim).filter(|line| !line.is_empty());
    lines.collect::<Vec<_>>().join("\n") + "\n"
}

/// How to rewrite the snapshots after a reviewed change.
const BLESS: &str = "QUECTO_BLESS_SNAPSHOTS=1 cargo test -p quecto-agentic-harness \
                     --test architecture project_board_telemetry </dev/null";

/// Each guarded file matches its snapshot. `QUECTO_BLESS_SNAPSHOTS=1`
/// (that value only, never in CI) rewrites them and fails, so a rewrite
/// is reviewed before the test passes.
#[test]
fn the_record_and_its_leaves_match_their_snapshots() {
    if std::env::var("QUECTO_BLESS_SNAPSHOTS").as_deref() == Ok("1") {
        assert!(
            std::env::var_os("CI").is_none(),
            "snapshots are never rewritten in CI"
        );
        for (file, snapshot) in SNAPSHOTS {
            let now = rendered(&std::fs::read_to_string(file).expect("a guarded file"));
            std::fs::write(snapshot, now).expect("write the snapshot");
        }
        panic!("snapshots rewritten: review the diff, then run again");
    }
    let changed: Vec<&str> = SNAPSHOTS
        .iter()
        .filter(|(file, snapshot)| {
            let now = rendered(&std::fs::read_to_string(file).expect("a guarded file"));
            std::fs::read_to_string(snapshot).unwrap_or_default() != now
        })
        .map(|(file, _)| *file)
        .collect();
    assert!(
        changed.is_empty(),
        "{changed:?} changed: review it, then run `{BLESS}`"
    );
}

/// A type declared in the record's module: a struct's or an enum's field
/// types, or a kind enum (`kinds!`, unit variants only).
enum Declared {
    Fields(Vec<syn::Type>),
    Kind,
}

fn declared(file: &syn::File) -> BTreeMap<String, Declared> {
    let mut types = BTreeMap::new();
    for item in &file.items {
        match item {
            syn::Item::Struct(item) => {
                let fields = item.fields.iter().map(|field| field.ty.clone()).collect();
                types.insert(item.ident.to_string(), Declared::Fields(fields));
            }
            syn::Item::Enum(item) => {
                let fields = item.variants.iter().flat_map(|v| v.fields.iter());
                let fields = fields.map(|field| field.ty.clone()).collect();
                types.insert(item.ident.to_string(), Declared::Fields(fields));
            }
            syn::Item::Macro(item) if item.mac.path.is_ident("kinds") => {
                let name = item
                    .mac
                    .tokens
                    .clone()
                    .into_iter()
                    .find_map(|token| match token {
                        TokenTree::Ident(ident) => Some(ident.to_string()),
                        _ => None,
                    });
                types.insert(name.expect("a kinds! enum has a name"), Declared::Kind);
            }
            _ => {}
        }
    }
    types
}

/// The types a field of `ty` holds, each of [`CONTAINERS`] unwrapped (by
/// its bare name only), each by its whole path (`other::StaticKind` is
/// not the leaf).
fn leaves(ty: &syn::Type, out: &mut Vec<String>) {
    let syn::Type::Path(path) = ty else {
        out.push(format!("<not a path: {}>", tokens(ty)));
        return;
    };
    let only = (path.qself.is_none() && path.path.segments.len() == 1)
        .then(|| &path.path.segments[0])
        .filter(|segment| CONTAINERS.iter().any(|name| segment.ident == name));
    match only.map(|segment| &segment.arguments) {
        Some(syn::PathArguments::AngleBracketed(arguments)) => {
            for argument in &arguments.args {
                match argument {
                    syn::GenericArgument::Type(inner) => leaves(inner, out),
                    other => out.push(format!("<argument: {}>", tokens(other))),
                }
            }
        }
        _ => out.push(tokens(&path.path).replace(' ', "")),
    }
}

/// Every type reachable from `root` that no rule admits.
fn refused_from(root: &str, types: &BTreeMap<String, Declared>) -> Vec<String> {
    let mut refused = Vec::new();
    let mut seen = vec![root.to_owned()];
    let mut queue = vec![root.to_owned()];
    while let Some(name) = queue.pop() {
        let fields = match types.get(&name) {
            Some(Declared::Fields(fields)) => fields,
            Some(Declared::Kind) => continue,
            None => {
                refused.push(name);
                continue;
            }
        };
        let mut found = Vec::new();
        for field in fields {
            leaves(field, &mut found);
        }
        for leaf in found {
            if !ADMITTED.contains(&leaf.as_str()) && !seen.contains(&leaf) {
                seen.push(leaf.clone());
                queue.push(leaf);
            }
        }
    }
    refused
}

#[test]
fn a_board_op_record_holds_only_kinds_counts_and_guarded_leaves() {
    let source = std::fs::read_to_string(RECORD_FILE).unwrap();
    let types = declared(&syn::parse_file(&source).unwrap());
    assert!(
        types.contains_key("BoardOpRecord"),
        "the record is in {RECORD_FILE}"
    );
    assert_eq!(refused_from("BoardOpRecord", &types), Vec::<String>::new());
}

#[test]
fn the_record_scan_refuses_text_fields() {
    let file: syn::File = syn::parse_quote! {
        pub struct BoardOpRecord { pub op: Inner, pub title: String, pub kind: other::StaticKind }
        pub struct Inner { pub path: Option<std::path::PathBuf>, pub ok: Option<u64> }
    };
    let refused = refused_from("BoardOpRecord", &declared(&file));
    assert_eq!(
        refused,
        ["other::StaticKind", "String", "std::path::PathBuf"]
    );
    // T4: text inside a list or a map is found, and only the bare
    // containers are read through.
    let nested: syn::File = syn::parse_quote! {
        pub struct BoardOpRecord {
            pub titles: Option<Vec<String>>,
            pub by_path: BTreeMap<std::path::PathBuf, u64>,
            pub by_kind: BTreeMap<StaticKind, Vec<i64>>,
            pub names: std::vec::Vec<StaticKind>,
            pub reasons: Vec<Box<str>>,
        }
    };
    let refused = refused_from("BoardOpRecord", &declared(&nested));
    assert_eq!(
        refused,
        [
            "Box<str>",
            "std::vec::Vec<StaticKind>",
            "std::path::PathBuf",
            "String"
        ]
    );
}

fn production_sources(dir: &Path, files: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            production_sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let file = path.display().to_string();
            if dependency_scan::production_file(&file) {
                files.push((file, std::fs::read_to_string(&path).expect("read")));
            }
        }
    }
}

fn sources() -> Vec<(String, String)> {
    let mut files = Vec::new();
    production_sources(Path::new("src"), &mut files);
    assert!(
        files.iter().any(|(file, _)| file == RECORD_FILE),
        "the scan reads src"
    );
    files
}

fn allowed(file: &str, paths: &[&str]) -> bool {
    paths
        .iter()
        .any(|path| file == *path || (path.ends_with('/') && file.starts_with(path)))
}

#[test]
fn the_log_and_the_switch_are_named_only_where_allowed() {
    let call = "fn f() { BoardRecording::when(switch, log); }";
    assert!(dependency_scan::forbidden_hit(call, &["BoardRecording::when"]).is_err());
    let files = sources();
    let mut stray = Vec::new();
    for (name, paths) in NAMED_ONLY_IN {
        let naming = files.iter().filter(|(file, source)| {
            !allowed(file, paths) && dependency_scan::forbidden_hit(source, &[name]).is_err()
        });
        stray.extend(naming.map(|(file, _)| format!("{name} in {file}")));
    }
    assert!(stray.is_empty(), "named outside NAMED_ONLY_IN: {stray:?}");
}

/// The names no public item in composition may carry in its signature: a
/// builder hands out a `BoardRecording`, never the bare port or adapter.
const NEVER_PUBLIC_IN_COMPOSITION: &[&str] = &[
    "ProjectBoardOpLog",
    "EventLogProjectBoardOps",
    "TracingProjectBoardOps",
];

fn mentions(item: &impl ToTokens) -> bool {
    let text = tokens(item);
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .collect();
    NEVER_PUBLIC_IN_COMPOSITION
        .iter()
        .any(|name| words.contains(name))
}

fn public(vis: &syn::Visibility) -> bool {
    !matches!(vis, syn::Visibility::Inherited)
}

/// The public signatures in a file that carry a
/// [`NEVER_PUBLIC_IN_COMPOSITION`] name.
#[derive(Default)]
struct Exposed(Vec<String>);

impl Exposed {
    fn check(&mut self, vis: &syn::Visibility, signature: &impl ToTokens) {
        if public(vis) && mentions(signature) {
            self.0.push(tokens(signature));
        }
    }
}

impl<'ast> Visit<'ast> for Exposed {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.check(&item.vis, &item.sig);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.check(&item.vis, &item.sig);
    }

    fn visit_field(&mut self, field: &'ast syn::Field) {
        self.check(&field.vis, &field.ty);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.check(&item.vis, &item.ty);
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        self.check(&item.vis, &item.ty);
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        self.check(&item.vis, &item.ty);
    }
}

fn exposed(file: &syn::File) -> Vec<String> {
    let mut found = Exposed::default();
    found.visit_file(file);
    found.0
}

#[test]
fn composition_hands_out_no_bare_port() {
    let bypass: syn::File = syn::parse_quote! {
        pub fn op_log(on: bool) -> Option<Arc<dyn ProjectBoardOpLog>> { None }
        pub struct Logs { pub log: EventLogProjectBoardOps }
        fn private(on: bool) -> Option<Arc<dyn ProjectBoardOpLog>> { None }
    };
    assert_eq!(exposed(&bypass).len(), 2);
    let found: Vec<String> = sources()
        .iter()
        .filter(|(file, _)| file.starts_with("src/composition/"))
        .flat_map(|(file, source)| {
            let found = exposed(&syn::parse_file(source).expect("parses"));
            found.into_iter().map(move |sig| format!("{file}: {sig}"))
        })
        .collect();
    assert!(
        found.is_empty(),
        "composition hands out the bare port: {found:?}"
    );
}

/// The board's own modules, its CLI and its composition: none writes a
/// `tracing` record but the adapters' file, whose adapters a recording
/// holds, so nothing is written while off.
const PROJECT_BOARD: &[&str] = &[
    "src/domain/project_board/",
    "src/application/project_board/",
    "src/interface/tools/project_board/",
    "src/interface/cli/board/",
    "src/infrastructure/persistence/project_board/",
    "src/composition/bootstrap/project_board.rs",
];

/// How a file reaches `tracing`: by its path, or under another name.
const TRACING: &[&str] = &["tracing::", "tracing as"];

#[test]
fn the_board_writes_no_tracing_record_but_through_its_adapters() {
    for traced in [
        "fn f() { tracing::info!(op = \"show\"); }",
        "use tracing as t; fn f() { t::info!(op = \"show\"); }",
        "extern crate tracing as t;",
    ] {
        assert!(
            dependency_scan::forbidden_hit(traced, TRACING).is_err(),
            "{traced}"
        );
    }
    let found: Vec<String> = sources()
        .iter()
        .filter(|(file, _)| allowed(file, PROJECT_BOARD) && file != ADAPTER)
        .filter_map(|(file, source)| {
            let hit = dependency_scan::forbidden_hit(source, TRACING).err();
            hit.map(|hit| format!("{file}: {hit}"))
        })
        .collect();
    assert!(found.is_empty(), "{found:?}");
}

/// The aliases of a [`NEVER_ALIASED`] name in a file.
#[derive(Default)]
struct Aliases(Vec<String>);

impl<'ast> Visit<'ast> for Aliases {
    fn visit_use_rename(&mut self, rename: &'ast syn::UseRename) {
        if NEVER_ALIASED.iter().any(|name| rename.ident == name) {
            self.0.push(tokens(rename));
        }
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        let aliased = tokens(&item.ty);
        let words: Vec<&str> = aliased
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .collect();
        if NEVER_ALIASED.iter().any(|name| words.contains(name)) {
            self.0.push(tokens(item));
        }
    }
}

fn aliases(file: &syn::File) -> Vec<String> {
    let mut found = Aliases::default();
    found.visit_file(file);
    found.0
}

#[test]
fn the_log_and_the_switch_are_never_aliased() {
    let bypass: syn::File = syn::parse_quote! {
        use crate::application::project_board::ports::ProjectBoardOpLog as Log;
        type Switch = crate::application::project_board::telemetry::RecordingSwitch;
    };
    assert_eq!(aliases(&bypass).len(), 2);
    let found: Vec<String> = sources()
        .iter()
        .flat_map(|(file, source)| {
            let found = aliases(&syn::parse_file(source).expect("parses"));
            found
                .into_iter()
                .map(move |alias| format!("{file}: {alias}"))
        })
        .collect();
    assert!(found.is_empty(), "aliased: {found:?}");
}

#[test]
fn every_board_op_has_its_kind_and_every_kind_its_op() {
    let ops: Vec<&str> = OPS.iter().map(|(name, _)| *name).collect();
    for op in &ops {
        assert_eq!(OpKind::named(op).as_str(), *op, "{op} has no OpKind");
    }
    for kind in OpKind::ALL.iter().filter(|kind| **kind != OpKind::Unknown) {
        assert!(
            ops.contains(&kind.as_str()),
            "{} is no op of OPS",
            kind.as_str()
        );
    }
}

/// Every name a record holds today from the board's own tables is a
/// known kind: the schema fields of `OPS`, the statuses, and the
/// pseudo-fields (`answer_tests.rs` holds every error and rule code).
#[test]
fn every_name_a_record_holds_is_a_known_kind() {
    let fields = OPS
        .iter()
        .flat_map(|(_, fields)| fields.iter().map(|field| field.name));
    let statuses = TaskStatus::ALL.iter().map(|status| status.as_str());
    let pseudo = ["op", "request", "other"];
    for name in fields.chain(statuses).chain(pseudo) {
        assert_eq!(
            StaticKind::new(name).as_str(),
            name,
            "{name} is no known kind"
        );
    }
}
