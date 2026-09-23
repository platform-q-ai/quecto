//! Shared scanning for the layer rules (#1637). Substring rules scan a file's
//! production tokens (`tests/common/production_tokens.rs`); path rules resolve
//! `super::`/`self::` against the file's place in the crate's module tree, so a
//! relative import is checked as the crate path it names.

#[path = "../common/production_tokens.rs"]
mod production_tokens;
pub(super) use production_tokens::{
    assert_no_forbidden, forbidden_hit, item_test_only, production_text, test_only,
    test_only_line_ranges,
};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// A crate source file's module path, from its crate's real module tree
/// (walked from each of [`CRATE_ROOTS`] through every `mod` declaration,
/// following `#[path]`): `src/infrastructure/tools/spawn.rs` →
/// `["infrastructure", "tools", "spawn"]`, and a `#[path]` child is named by
/// the module that declares it, not by where its file sits. A file outside the
/// tree is a caller bug.
pub(super) fn module_path(file: &str) -> Vec<String> {
    static TREE: OnceLock<BTreeMap<PathBuf, Vec<String>>> = OnceLock::new();
    let tree = TREE.get_or_init(|| {
        let mut tree = BTreeMap::new();
        // The harness (the suite runs from its package directory) and the
        // TUI, whose layer rules this suite also enforces.
        for root in CRATE_ROOTS {
            walk_module(Path::new(root), true, Vec::new(), &mut tree);
        }
        tree
    });
    tree.get(&normalize(Path::new(file)))
        .cloned()
        .unwrap_or_else(|| panic!("{file} is not in the crate module tree"))
}

/// The crate roots whose module trees the path rules resolve against.
const CRATE_ROOTS: [&str; 2] = ["src/lib.rs", "../quecto-tui/src/lib.rs"];

/// Lexically resolve `.`/`..` so `#[path = "../x.rs"]` keys match.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `mod_rs`: the file names its own directory for child modules (`lib.rs`,
/// `mod.rs`, or a file loaded by `#[path]`); otherwise children live in the
/// directory named after the file.
fn walk_module(
    file: &Path,
    mod_rs: bool,
    module: Vec<String>,
    tree: &mut BTreeMap<PathBuf, Vec<String>>,
) {
    let key = normalize(file);
    if let Some(mounted) = tree.get(&key) {
        // One file, one module: a second mount would make `super::` mean
        // two things, so the scan refuses it rather than pick one.
        assert_eq!(
            mounted,
            &module,
            "{} is mounted as two modules",
            file.display()
        );
        return;
    }
    let file_dir = file.parent().expect("module file has a directory");
    let child_dir = if mod_rs {
        file_dir.to_path_buf()
    } else {
        file_dir.join(file.file_stem().expect("module file has a stem"))
    };
    mount(file, key, file_dir, &child_dir, module, tree);
}

/// Record `file` as `module` and walk its declarations.
fn mount(
    file: &Path,
    key: PathBuf,
    path_base: &Path,
    child_dir: &Path,
    module: Vec<String>,
    tree: &mut BTreeMap<PathBuf, Vec<String>>,
) {
    let source = std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("read module file {}: {e}", file.display()));
    let parsed = syn::parse_file(&source)
        .unwrap_or_else(|e| panic!("parse module file {}: {e}", file.display()));
    tree.insert(key, module.clone());
    walk_items(&parsed.items, path_base, child_dir, &module, tree);
}

fn walk_items(
    items: &[syn::Item],
    path_base: &Path,
    child_dir: &Path,
    module: &[String],
    tree: &mut BTreeMap<PathBuf, Vec<String>>,
) {
    for item in items {
        // `include!("x.rs")` splices the file into the including module.
        if let syn::Item::Macro(included) = item
            && included.mac.path.is_ident("include")
        {
            let target: syn::LitStr = included
                .mac
                .parse_body()
                .unwrap_or_else(|e| panic!("include! without a literal path: {e}"));
            let file = path_base.join(target.value());
            let key = normalize(&file);
            if let Some(mounted) = tree.get(&key) {
                assert_eq!(
                    mounted,
                    module,
                    "{} is mounted as two modules",
                    file.display()
                );
                continue;
            }
            mount(&file, key, path_base, child_dir, module.to_vec(), tree);
            continue;
        }
        let syn::Item::Mod(declared) = item else {
            continue;
        };
        let name = declared.ident.to_string();
        let mut child = module.to_vec();
        child.push(name.clone());
        let explicit = path_attribute(&declared.attrs);
        if let Some((_, inline)) = &declared.content {
            // Declarations inside an inline module resolve against its own
            // directory: `#[path = "d"] mod m { … }` names `d`, otherwise
            // `m` under the enclosing directory.
            let dir = match &explicit {
                Some(path) => path_base.join(path),
                None => child_dir.join(&name),
            };
            walk_items(inline, &dir, &dir, &child, tree);
            continue;
        }
        match explicit {
            Some(path) => walk_module(&path_base.join(path), true, child, tree),
            None => {
                let flat = child_dir.join(format!("{name}.rs"));
                let nested = child_dir.join(&name).join("mod.rs");
                if flat.is_file() {
                    walk_module(&flat, false, child, tree);
                } else if nested.is_file() {
                    walk_module(&nested, true, child, tree);
                }
                // Neither: a declaration for another target; nothing to map.
            }
        }
    }
}

/// The `#[path = "…"]` value, if any.
fn path_attribute(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| match &attr.meta {
        syn::Meta::NameValue(value) if value.path.is_ident("path") => match &value.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(path),
                ..
            }) => Some(path.value()),
            _ => None,
        },
        _ => None,
    })
}

/// Marker for a path that climbs above the crate root or aliases a module
/// (`use crate as root;`): no layer rule admits it.
pub(super) const UNRESOLVABLE: &str = "crate::<unresolvable module path>";

/// Rewrite a leading `super::`/`self::` chain into the crate path it names
/// from `module`; other paths are returned unchanged.
pub(super) fn resolve_relative(module: &[String], path: &str) -> String {
    let segments: Vec<&str> = path.split("::").collect();
    if !matches!(segments.first(), Some(&"super") | Some(&"self")) {
        return path.to_string();
    }
    let mut base = module.to_vec();
    let mut rest = segments.as_slice();
    while let Some((&head, tail)) = rest.split_first() {
        match head {
            "self" => {}
            "super" => {
                if base.pop().is_none() {
                    return UNRESOLVABLE.to_string();
                }
            }
            _ => break,
        }
        rest = tail;
    }
    std::iter::once("crate")
        .chain(base.iter().map(String::as_str))
        .chain(rest.iter().copied())
        .collect::<Vec<_>>()
        .join("::")
}

/// Crate-relative paths (`crate::…`, `$crate::…`, `super::…`, `self::…`)
/// spelled in raw tokens — macro arguments and `macro_rules!` bodies, which
/// `syn` leaves unparsed. A lone `self`/`super` (`self.field`) is no path.
pub(super) fn crate_paths_in_tokens(stream: proc_macro2::TokenStream) -> Vec<String> {
    use proc_macro2::TokenTree;
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut paths = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if let TokenTree::Group(group) = &tokens[i] {
            paths.extend(crate_paths_in_tokens(group.stream()));
            i += 1;
            continue;
        }
        let dollar = matches!(&tokens[i], TokenTree::Punct(p) if p.as_char() == '$');
        let head = if dollar { i + 1 } else { i };
        let start = match tokens.get(head) {
            Some(TokenTree::Ident(ident))
                if (dollar && ident == "crate")
                    || (!dollar
                        && matches!(ident.to_string().as_str(), "crate" | "super" | "self")) =>
            {
                ident.to_string()
            }
            _ => {
                i += 1;
                continue;
            }
        };
        let mut segments = vec![start];
        let mut next = head + 1;
        while let (
            Some(TokenTree::Punct(a)),
            Some(TokenTree::Punct(b)),
            Some(TokenTree::Ident(ident)),
        ) = (tokens.get(next), tokens.get(next + 1), tokens.get(next + 2))
        {
            if a.as_char() != ':' || b.as_char() != ':' {
                break;
            }
            segments.push(ident.to_string());
            next += 3;
        }
        if segments.len() > 1 {
            paths.push(segments.join("::"));
        }
        i = next.max(i + 1);
    }
    paths
}

/// Crate-relative paths spelled as string literals in tokens (attribute
/// arguments such as `#[serde(with = "crate::codec")]`).
pub(super) fn string_paths_in_tokens(stream: proc_macro2::TokenStream) -> Vec<String> {
    use proc_macro2::TokenTree;
    let mut paths = Vec::new();
    for token in stream {
        match token {
            TokenTree::Group(group) => paths.extend(string_paths_in_tokens(group.stream())),
            TokenTree::Literal(literal) => {
                let Ok(text) = syn::parse_str::<syn::LitStr>(&literal.to_string()) else {
                    continue;
                };
                let value = text.value();
                if let Ok(path) = syn::parse_str::<syn::Path>(&value)
                    && path.segments.len() > 1
                    && path.segments.first().is_some_and(|first| {
                        matches!(first.ident.to_string().as_str(), "crate" | "super" | "self")
                    })
                {
                    paths.push(value.replace(' ', ""));
                }
            }
            _ => {}
        }
    }
    paths
}

/// True when every segment names a module relative to the crate (`crate`,
/// `super`, `self`): renaming such a path creates a module alias the path
/// rules could not follow.
pub(super) fn is_module_alias(path: &str) -> bool {
    path.split("::")
        .all(|segment| matches!(segment, "crate" | "super" | "self"))
}

#[test]
fn production_text_keeps_items_after_a_test_module() {
    let source = "#[cfg(test)]\nmod tests;\n\npub use crate::application::secret::Bad;\n";
    let text = production_text(source).expect("parses");
    assert!(text.contains("crate::application::secret::Bad"), "{text}");
    assert!(!text.contains("tests"), "{text}");
}

#[test]
fn production_text_drops_test_only_items_anywhere() {
    for source in [
        "#[cfg(test)]\nmod tests { use std::fs::read; }\nfn ok() {}",
        "struct S; impl S { #[cfg(test)] fn t() { std::fs::read(\"x\"); } }",
        "fn f() { #[cfg(test)] fn t() { std::fs::read(\"x\"); } }",
        "mod inner { #[cfg(test)] use std::fs::read; }",
    ] {
        let text = production_text(source).expect("parses");
        assert!(!text.contains("std::fs::"), "{source}: {text}");
    }
}

#[test]
fn production_text_ignores_comments_and_literals() {
    for source in [
        "// use crate::application::secret::Bad;\nfn ok() {}",
        "/// crate::application::secret::Bad\nfn ok() {}",
        "#![doc = \"crate::application::secret::Bad\"]\nfn ok() {}",
        "/* std::fs::read */ fn ok() {}",
        "fn ok() { let _ = \"std::fs::read\"; }",
        "fn ok() { println!(\"crate::application::x\"); }",
    ] {
        let text = production_text(source).expect("parses");
        assert!(
            !text.contains("std::fs::") && !text.contains("crate::application"),
            "{source}: {text}"
        );
    }
}

#[test]
fn production_text_keeps_cfg_combinations_and_multiline_paths() {
    for (source, pattern) in [
        (
            "#[cfg(any(test, feature = \"x\"))]\nuse std::fs::read;",
            "std::fs::",
        ),
        (
            "use crate::\n    application::\n    secret::Bad;",
            "crate::application",
        ),
        ("fn f(p: &Path) -> bool { p\n    .exists() }", ".exists("),
        ("fn f() { Command::new(\"x\"); }", "Command::new"),
        // Only `cfg(test)` is test-only; other cfg-gated items are production.
        ("#[cfg(unix)]\nuse std::fs::read;", "std::fs::"),
        // A path spelled in an attribute string still names a dependency.
        (
            "#[serde(with = \"crate::infrastructure::codec\")]\nstruct S;",
            "crate::infrastructure",
        ),
    ] {
        let text = production_text(source).expect("parses");
        assert!(text.contains(pattern), "{source}: {text}");
    }
    assert!(production_text("fn broken( {").is_none());
    assert!(forbidden_hit("fn broken( {", &["x"]).is_err());
}

#[test]
fn module_paths_follow_the_module_tree() {
    assert_eq!(
        module_path("src/infrastructure/tools/spawn.rs"),
        ["infrastructure", "tools", "spawn"]
    );
    assert_eq!(module_path("src/domain/mod.rs"), ["domain"]);
    assert!(module_path("src/lib.rs").is_empty());
    // A `#[path]` child is named by the module that declares it.
    assert_eq!(
        module_path("src/application/agent_loop_turn_flow.rs"),
        ["application", "agent_loop", "agent_loop_turn_flow"]
    );
    assert_eq!(
        normalize(Path::new("src/a/../b/./c.rs")),
        Path::new("src/b/c.rs")
    );
}

/// Walk a synthetic crate rooted at `lib` (files: relative path → source).
fn walk_fixture(files: &[(&str, &str)]) -> BTreeMap<PathBuf, Vec<String>> {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, source) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let mut tree = BTreeMap::new();
    walk_module(&dir.path().join("lib.rs"), true, Vec::new(), &mut tree);
    tree.into_iter()
        .map(|(path, module)| (path.strip_prefix(dir.path()).unwrap().to_path_buf(), module))
        .collect()
}

#[test]
fn walker_follows_inline_paths_includes_and_non_mod_rs_children() {
    let tree = walk_fixture(&[
        (
            "lib.rs",
            "#[path = \"d\"] mod m { mod c; }\nmod a;\ninclude!(\"spliced.rs\");",
        ),
        ("d/c.rs", ""),
        ("a.rs", "mod b;"),
        ("a/b.rs", ""),
        ("spliced.rs", "mod s;"),
        ("s.rs", ""),
    ]);
    let module = |path: &str| tree.get(Path::new(path)).cloned();
    assert_eq!(
        module("d/c.rs"),
        Some(vec!["m".to_string(), "c".to_string()])
    );
    assert_eq!(
        module("a/b.rs"),
        Some(vec!["a".to_string(), "b".to_string()])
    );
    assert_eq!(module("spliced.rs"), Some(vec![]));
    assert_eq!(module("s.rs"), Some(vec!["s".to_string()]));
}

#[test]
#[should_panic(expected = "is mounted as two modules")]
fn walker_refuses_a_file_mounted_twice() {
    walk_fixture(&[
        (
            "lib.rs",
            "#[path = \"x.rs\"] mod one;\n#[path = \"x.rs\"] mod two;",
        ),
        ("x.rs", ""),
    ]);
}

#[test]
#[should_panic(expected = "is not in the crate module tree")]
fn files_outside_the_module_tree_are_refused() {
    module_path("src/not_a_module.rs");
}

#[test]
fn relative_paths_resolve_to_crate_paths() {
    let module = module_path("src/infrastructure/tools/spawn.rs");
    for (path, expected) in [
        (
            "super::spawn_entry::X",
            "crate::infrastructure::tools::spawn_entry::X",
        ),
        (
            "self::inner::Y",
            "crate::infrastructure::tools::spawn::inner::Y",
        ),
        (
            "super::super::super::application::secret::Bad",
            "crate::application::secret::Bad",
        ),
        ("super::super::super::super::x", UNRESOLVABLE),
        ("crate::domain::X", "crate::domain::X"),
        ("std::fs::read", "std::fs::read"),
        ("super", "crate::infrastructure::tools"),
    ] {
        assert_eq!(resolve_relative(&module, path), expected, "{path}");
    }
    assert!(is_module_alias("crate"));
    assert!(is_module_alias("super::super"));
    assert!(is_module_alias("crate::self"));
    assert!(!is_module_alias("crate::application"));
    assert!(!is_module_alias("super::spawn::self"));
}

#[test]
fn crate_paths_are_read_out_of_macro_tokens() {
    let tokens: proc_macro2::TokenStream =
        "vec![crate::application::x::Y::new(), $crate::domain::Z, \
         super::super::infra::W, self.field, self::local, \"crate::in::string\"]"
            .parse()
            .unwrap();
    assert_eq!(
        crate_paths_in_tokens(tokens),
        [
            "crate::application::x::Y::new",
            "crate::domain::Z",
            "super::super::infra::W",
            "self::local",
        ]
    );
    let attr: proc_macro2::TokenStream = "serde(with = \"crate::infrastructure::codec\", \
         rename = \"camelCase\", default = \"Default::default\")"
        .parse()
        .unwrap();
    assert_eq!(
        string_paths_in_tokens(attr),
        ["crate::infrastructure::codec"]
    );
}

#[test]
#[should_panic(expected = "policy x: forbidden pattern crate::shell")]
fn assert_no_forbidden_names_the_rule_and_pattern() {
    assert_no_forbidden("policy x", "fn ok() {}", &["crate::shell"]);
    assert_no_forbidden("policy x", "use crate::shell::App;", &["crate::shell"]);
}
