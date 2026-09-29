//! A board use case is metered whole (#2303 round-4 review L4): the
//! dispatcher serves a metered call over [`OverRepository::over`], which
//! swaps the use case's one repository for the call's own. That measures
//! every transaction only while the use case holds exactly one
//! `Arc<dyn BoardRepository>` and nests no other board use case (whose
//! repository `over` would leave unmetered). So every use case (a struct
//! with an `impl OverRepository`) holds exactly one repository field, and
//! every other field is one of the [`OTHER_PORTS`] allowlist: a nested use
//! case, a second repository, or a repository behind any other type fails.
//!
//! A use case that runs another's work (S6's `JoinRun`, which runs
//! `_admit`'s and `_activate`'s) calls that work's shared function over its
//! own repository (`admit_member::admit`, `activate_member::activate`),
//! never a nested use case, so `over` meters every transaction of the call
//! and no use case is allowlisted here.
use std::collections::BTreeSet;
use std::path::Path;

use quote::ToTokens;

use super::dependency_scan;

/// The directory holding the board's use cases, one per file.
const USE_CASES: &str = "src/application/swarm/use_cases";

/// The repository field every use case holds exactly once.
const REPOSITORY: &str = "Arc < dyn BoardRepository >";

/// The only other fields a use case may hold: ports composition binds and
/// `over` carries across unchanged, none of which runs a transaction.
const OTHER_PORTS: &[&str] = &[
    "Arc < dyn BoardEncoding >",
    "Arc < dyn Clock + Send + Sync >",
    "Arc < dyn IdSource >",
];

fn tokens(tokens: &impl ToTokens) -> String {
    tokens
        .to_token_stream()
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The name of an `impl OverRepository for <name>`, when `item` is one.
fn over_repository_for(item: &syn::Item) -> Option<String> {
    let syn::Item::Impl(item) = item else {
        return None;
    };
    let (_, trait_path, _) = item.trait_.as_ref()?;
    let named = trait_path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "OverRepository");
    match (named, &*item.self_ty) {
        (true, syn::Type::Path(path)) => path.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

/// What breaks the rule in `sources` (each a use-case file's text): one
/// line per use case whose fields are not exactly one repository and
/// otherwise [`OTHER_PORTS`]. Also returns the use cases found.
fn violations(sources: &[&str]) -> (Vec<String>, BTreeSet<String>) {
    let files: Vec<syn::File> = sources
        .iter()
        .map(|source| syn::parse_file(source).expect("a use-case source parses"))
        .collect();
    let use_cases: BTreeSet<String> = files
        .iter()
        .flat_map(|file| file.items.iter().filter_map(over_repository_for))
        .collect();
    let mut found = Vec::new();
    let mut checked = BTreeSet::new();
    for item in files.iter().flat_map(|file| &file.items) {
        let syn::Item::Struct(item) = item else {
            continue;
        };
        let name = item.ident.to_string();
        if !use_cases.contains(&name) {
            continue;
        }
        checked.insert(name.clone());
        let syn::Fields::Named(fields) = &item.fields else {
            found.push(format!("{name}: a use case names its fields"));
            continue;
        };
        let mut repositories = 0;
        for field in &fields.named {
            let field_type = tokens(&field.ty);
            let field_name = field
                .ident
                .as_ref()
                .map_or_else(String::new, ToString::to_string);
            let nested = use_cases.iter().find(|use_case| {
                field_type
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|word| word == use_case.as_str())
            });
            match (field_type.as_str(), nested) {
                (REPOSITORY, _) => repositories += 1,
                (_, Some(nested)) => found.push(format!(
                    "{name}.{field_name} nests the board use case {nested}: `over` would leave its repository unmetered"
                )),
                (other, None) if OTHER_PORTS.contains(&other) => {}
                (other, None) => found.push(format!(
                    "{name}.{field_name}: `{other}` is neither its repository nor an allowlisted port"
                )),
            }
        }
        if repositories != 1 {
            found.push(format!(
                "{name} holds {repositories} `{REPOSITORY}` fields, not exactly one"
            ));
        }
    }
    let unseen: Vec<_> = use_cases.difference(&checked).collect();
    assert!(
        unseen.is_empty(),
        "use cases with no struct here: {unseen:?}"
    );
    (found, use_cases)
}

/// The production use-case files.
fn use_case_sources() -> Vec<String> {
    let mut sources = Vec::new();
    for entry in std::fs::read_dir(USE_CASES).expect("read the use cases") {
        let path = entry.expect("dir entry").path();
        let file = path.display().to_string();
        let rust = path.extension().is_some_and(|ext| ext == "rs");
        if rust && dependency_scan::production_file(&file) {
            sources.push(std::fs::read_to_string(Path::new(&file)).expect("read a use case"));
        }
    }
    sources
}

#[test]
fn each_board_use_case_holds_one_repository_and_no_use_case() {
    let sources = use_case_sources();
    let borrowed: Vec<&str> = sources.iter().map(String::as_str).collect();
    let (found, use_cases) = violations(&borrowed);
    assert!(
        use_cases.len() >= 4,
        "the scan reads the board's use cases: {use_cases:?}"
    );
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_rule_refuses_a_second_repository_a_nested_use_case_and_any_other_field() {
    let use_case = |name: &str, fields: &str| {
        format!(
            "pub struct {name} {{ {fields} }}\n\
             impl OverRepository for {name} {{ fn over(&self, r: Arc<dyn BoardRepository>) -> Self {{ Self::new(r) }} }}"
        )
    };
    let plain = use_case(
        "Plain",
        "repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>, ids: Arc<dyn IdSource>",
    );
    let (found, use_cases) = violations(&[&plain, "pub struct Request { member: String }"]);
    assert_eq!(found, Vec::<String>::new());
    assert_eq!(use_cases.into_iter().collect::<Vec<_>>(), ["Plain"]);

    for (fields, expected) in [
        (
            "repository: Arc<dyn BoardRepository>, admit: Arc<Plain>",
            "Outer.admit nests the board use case Plain",
        ),
        (
            "repository: Arc<dyn BoardRepository>, admit: Plain",
            "Outer.admit nests the board use case Plain",
        ),
        (
            "repository: Arc<dyn BoardRepository>, other: Arc<dyn BoardRepository>",
            "Outer holds 2 `Arc < dyn BoardRepository >` fields",
        ),
        (
            "clock: Arc<dyn Clock + Send + Sync>",
            "Outer holds 0 `Arc < dyn BoardRepository >` fields",
        ),
        (
            "repository: Arc<dyn BoardRepository>, cached: Option<Arc<dyn BoardRepository>>",
            "Outer.cached: `Option < Arc < dyn BoardRepository > >` is neither",
        ),
        (
            "repository: Arc<dyn BoardRepository>, meter: Arc<dyn BoardCallMeter>",
            "Outer.meter: `Arc < dyn BoardCallMeter >` is neither",
        ),
    ] {
        let outer = use_case("Outer", fields);
        let (found, _) = violations(&[&plain, &outer]);
        assert!(
            found.iter().any(|line| line.starts_with(expected)),
            "{fields}: {found:?}"
        );
    }
    let tuple = "pub struct Tuple(Arc<dyn BoardRepository>);\n\
                 impl OverRepository for Tuple { fn over(&self, r: Arc<dyn BoardRepository>) -> Self { Self::new(r) } }";
    let (found, _) = violations(&[tuple]);
    assert_eq!(found, ["Tuple: a use case names its fields"]);
}
