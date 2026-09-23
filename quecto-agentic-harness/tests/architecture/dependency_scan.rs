//! Shared scanning for the layer rules (#1637). Substring rules scan a file's
//! production tokens (`tests/common/production_tokens.rs`); path rules resolve
//! `super::`/`self::` against the file's place in the crate's module tree, so a
//! relative import is checked as the crate path it names.

#[path = "../common/production_tokens.rs"]
mod production_tokens;
pub(super) use production_tokens::{
    assert_no_forbidden, forbidden_hit, item_test_only, production_lines, production_text,
    test_only,
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
    assert!(
        !test_only_file(file),
        "{file} is mounted only under #[cfg(test)]"
    );
    crate_tree().production[&normalize(Path::new(file))].clone()
}

/// True when `file` is mounted only under `#[cfg(test)]` (test code the
/// layer rules skip, like `*_tests.rs`); false for a production module. A
/// file in neither tree (not compiled, or a caller bug) is refused.
pub(super) fn test_only_file(file: &str) -> bool {
    let key = normalize(Path::new(file));
    let tree = crate_tree();
    if tree.production.contains_key(&key) {
        return false;
    }
    assert!(
        tree.test_only.contains(&key),
        "{file} is not in the crate module tree"
    );
    true
}

/// The files the crate roots mount: production modules with their module
/// paths, and files reached only through `#[cfg(test)]` declarations.
#[derive(Default)]
struct Tree {
    production: BTreeMap<PathBuf, Vec<String>>,
    test_only: std::collections::BTreeSet<PathBuf>,
}

fn crate_tree() -> &'static Tree {
    static TREE: OnceLock<Tree> = OnceLock::new();
    TREE.get_or_init(|| {
        let mut tree = Tree::default();
        // The harness (the suite runs from its package directory) and the
        // TUI, whose layer rules this suite also enforces.
        for root in CRATE_ROOTS {
            walk_module(Path::new(root), true, Vec::new(), false, &mut tree);
        }
        tree
    })
}

/// The layer rules skip `*_tests.rs` files by name, so a production module
/// must never be one: every such file is mounted only under `#[cfg(test)]`.
#[test]
fn production_modules_are_not_test_files() {
    let tree = &crate_tree().production;
    assert!(tree.len() > 500, "both crates were walked ({})", tree.len());
    let named_as_tests: Vec<_> = tree
        .keys()
        .filter(|file| {
            file.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_tests.rs"))
        })
        .collect();
    assert!(
        named_as_tests.is_empty(),
        "production *_tests.rs: {named_as_tests:?}"
    );
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
fn walk_module(file: &Path, mod_rs: bool, module: Vec<String>, test: bool, tree: &mut Tree) {
    let file_dir = file.parent().expect("module file has a directory");
    let child_dir = if mod_rs {
        file_dir.to_path_buf()
    } else {
        file_dir.join(file.file_stem().expect("module file has a stem"))
    };
    mount(file, file_dir, &child_dir, module, test, tree);
}

/// Record `file` as `module` and walk its declarations. One file, one
/// module: a second mount would make `super::` mean two things, so the scan
/// refuses it rather than pick one.
/// `test`: reached through a `#[cfg(test)]` declaration — recorded apart,
/// and free to share a file (two test modules may mount one fixture).
fn mount(
    file: &Path,
    path_base: &Path,
    child_dir: &Path,
    module: Vec<String>,
    test: bool,
    tree: &mut Tree,
) {
    let key = normalize(file);
    if let Some(mounted) = tree.production.get(&key) {
        // A test module may mount a production file as a fixture.
        if !test {
            assert_eq!(
                mounted,
                &module,
                "{} is mounted as two modules",
                file.display()
            );
        }
        return;
    }
    if test && !tree.test_only.insert(key.clone()) {
        return;
    }
    let source = std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("read module file {}: {e}", file.display()));
    let parsed = syn::parse_file(&source)
        .unwrap_or_else(|e| panic!("parse module file {}: {e}", file.display()));
    if !test {
        tree.test_only.remove(&key);
        tree.production.insert(key, module.clone());
    }
    let dirs = Dirs {
        include_base: file.parent().expect("module file has a directory"),
        path_base,
        child_dir,
    };
    walk_items(&parsed.items, &dirs, &module, test, tree);
}

/// Where a file's declarations resolve (Rust reference, "Modules"):
/// `include!` against the file holding the macro; `#[path]` on a declaration
/// against `path_base` (the file's directory, or an inline module's); an
/// implicit `mod x;` under `child_dir`.
struct Dirs<'a> {
    include_base: &'a Path,
    path_base: &'a Path,
    child_dir: &'a Path,
}

fn walk_items(
    items: &[syn::Item],
    dirs: &Dirs<'_>,
    module: &[String],
    test: bool,
    tree: &mut Tree,
) {
    for item in items {
        let test = test || item_test_only(item);
        // `include!("x.rs")` splices the file into the including module.
        if let syn::Item::Macro(included) = item
            && included.mac.path.is_ident("include")
        {
            let target: syn::LitStr = included
                .mac
                .parse_body()
                .unwrap_or_else(|e| panic!("include! without a literal path: {e}"));
            let file = dirs.include_base.join(target.value());
            mount(
                &file,
                dirs.path_base,
                dirs.child_dir,
                module.to_vec(),
                test,
                tree,
            );
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
            // An inline module's declarations resolve under the enclosing
            // module's directory (the file-stem directory in a non-mod-rs
            // file): `#[path = "d"] mod m { … }` names `d`, otherwise `m`.
            let dir = dirs.child_dir.join(explicit.as_deref().unwrap_or(&name));
            let inner = Dirs {
                include_base: dirs.include_base,
                path_base: &dir,
                child_dir: &dir,
            };
            walk_items(inline, &inner, &child, test, tree);
            continue;
        }
        let candidates = match &explicit {
            Some(path) => vec![(dirs.path_base.join(path), true)],
            None => vec![
                (dirs.child_dir.join(format!("{name}.rs")), false),
                (dirs.child_dir.join(&name).join("mod.rs"), true),
            ],
        };
        // No file: a declaration for another target; nothing to map (a
        // scanned file outside the tree is refused by `module_path`).
        if let Some((file, mod_rs)) = candidates.into_iter().find(|(file, _)| file.is_file()) {
            walk_module(&file, mod_rs, child, test, tree);
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

/// Crate-relative paths (`crate::…`, `super::…`, `self::…`; `$crate` is the
/// `$` punct then the `crate` ident) spelled in raw tokens — macro arguments
/// and `macro_rules!` bodies, which `syn` leaves unparsed. A lone
/// `self`/`super` (`self.field`) is no path.
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
        // `use dirs as d;` / `extern crate std as s;` inside macro tokens
        // alias a crate the text rules could not follow.
        let word = |at: usize, text: &str| matches!(tokens.get(at), Some(TokenTree::Ident(w)) if w == text);
        // `crate`/`super`/`self` aliases are the chain's case below.
        let lower = |at: usize| {
            matches!(tokens.get(at), Some(TokenTree::Ident(w))
                if w.to_string().starts_with(|c: char| c.is_ascii_lowercase())
                    && !matches!(w.to_string().as_str(), "crate" | "super" | "self"))
        };
        if (word(i, "use") && lower(i + 1) && word(i + 2, "as"))
            || (word(i, "extern") && word(i + 1, "crate") && word(i + 3, "as"))
        {
            paths.push(UNRESOLVABLE.to_string());
            i += 1;
            continue;
        }
        let start = match tokens.get(i) {
            Some(TokenTree::Ident(ident))
                if matches!(ident.to_string().as_str(), "crate" | "super" | "self") =>
            {
                ident.to_string()
            }
            _ => {
                i += 1;
                continue;
            }
        };
        let mut segments = vec![start];
        let mut next = i + 1;
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
        let joint = |at: usize| {
            matches!((tokens.get(at), tokens.get(at + 1)),
                (Some(TokenTree::Punct(a)), Some(TokenTree::Punct(b)))
                    if a.as_char() == ':' && b.as_char() == ':')
        };
        match tokens.get(next + 2) {
            // `crate::{a::X, b as c}`: expand the group as a use tree.
            Some(TokenTree::Group(group))
                if joint(next) && group.delimiter() == proc_macro2::Delimiter::Brace =>
            {
                let tree = format!("{}::{}", segments.join("::"), group);
                match syn::parse_str::<syn::UseTree>(&tree) {
                    Ok(tree) => expand_use_tree(&tree, "", &mut paths),
                    Err(_) => paths.push(UNRESOLVABLE.to_string()),
                }
                next += 3;
            }
            // `crate as c` / `super::super as up` inside a macro.
            _ if matches!(tokens.get(next), Some(TokenTree::Ident(word)) if word == "as")
                && is_module_alias(&segments.join("::")) =>
            {
                paths.push(UNRESOLVABLE.to_string());
            }
            _ if segments.len() > 1 => paths.push(segments.join("::")),
            _ => {}
        }
        i = next.max(i + 1);
    }
    paths
}

/// The leaf paths of a use tree; a module alias is [`UNRESOLVABLE`].
fn expand_use_tree(tree: &syn::UseTree, prefix: &str, paths: &mut Vec<String>) {
    match tree {
        syn::UseTree::Path(path) => {
            expand_use_tree(&path.tree, &format!("{prefix}{}::", path.ident), paths)
        }
        syn::UseTree::Name(name) => paths.push(format!("{prefix}{}", name.ident)),
        syn::UseTree::Rename(name) if is_module_alias(&format!("{prefix}{}", name.ident)) => {
            paths.push(UNRESOLVABLE.to_string())
        }
        syn::UseTree::Rename(name) => paths.push(format!("{prefix}{}", name.ident)),
        syn::UseTree::Glob(_) => paths.push(format!("{prefix}*")),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                expand_use_tree(item, prefix, paths);
            }
        }
    }
}

/// Crate-relative paths in the code an attribute holds as strings — the
/// path-valued keys (`#[serde(with = "crate::codec")]`, `try_from =
/// "Vec<crate::X>"`, `bound = "T: crate::Tr"`); prose strings are ignored.
pub(super) fn string_paths_in_tokens(stream: proc_macro2::TokenStream) -> Vec<String> {
    string_paths_in(stream, false)
}

fn string_paths_in(stream: proc_macro2::TokenStream, in_bound: bool) -> Vec<String> {
    use proc_macro2::TokenTree;
    use syn::parse::Parser;
    use syn::visit::Visit;
    struct Paths(Vec<String>);
    impl<'ast> Visit<'ast> for Paths {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segments.len() > 1 && matches!(segments[0].as_str(), "crate" | "super" | "self") {
                self.0.push(segments.join("::"));
            }
            syn::visit::visit_path(self, path);
        }
    }
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut paths = Paths(Vec::new());
    for (at, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Group(group) => paths.0.extend(string_paths_in(
                group.stream(),
                production_tokens::opens_bound(&tokens, at),
            )),
            TokenTree::Literal(literal)
                if production_tokens::is_path_valued(&tokens, at, in_bound) =>
            {
                let Ok(text) = syn::parse_str::<syn::LitStr>(&literal.to_string()) else {
                    continue;
                };
                let code = text.value();
                if let Ok(ty) = syn::parse_str::<syn::Type>(&code) {
                    paths.visit_type(&ty);
                } else if let Ok(predicates) = syn::punctuated::Punctuated::<
                    syn::WherePredicate,
                    syn::Token![,],
                >::parse_terminated
                    .parse_str(&code)
                {
                    for predicate in &predicates {
                        paths.visit_where_predicate(predicate);
                    }
                } else if code.contains("crate") || code.contains("super") || code.contains("self")
                {
                    // Unparseable code that may name the crate fails closed.
                    paths.0.push(UNRESOLVABLE.to_string());
                }
            }
            _ => {}
        }
    }
    paths.0
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
        "#[derive(Debug)]\n#[error(\"std::fs::read failed in crate::application\")]\nstruct E;",
        "#[expect(clippy::x, reason = \"calls .exists( via std::fs::\")]\nfn ok() {}",
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
        (
            "#[serde(try_from = \"Vec<crate::interface::X>\")]\nstruct S;",
            "crate::interface",
        ),
    ] {
        let text = production_text(source).expect("parses");
        assert!(text.contains(pattern), "{source}: {text}");
    }
    assert!(production_text("fn broken( {").is_none());
    // Name-led patterns match at a name boundary; others anywhere.
    assert!(forbidden_hit("use crate::domain::environment_dirs::X;", &["dirs::"]).is_ok());
    assert!(forbidden_hit("fn f() { HttpClient::new(); }", &["Client::"]).is_ok());
    assert!(forbidden_hit("fn f() { dirs::home_dir(); }", &["dirs::"]).is_err());
    assert!(forbidden_hit("fn f(p: P) { p.exists(); }", &[".exists("]).is_err());
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
    walk_fixture_tree(files).production
}

fn walk_fixture_tree(files: &[(&str, &str)]) -> Tree {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, source) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let mut tree = Tree::default();
    walk_module(
        &dir.path().join("lib.rs"),
        true,
        Vec::new(),
        false,
        &mut tree,
    );
    let relative = |path: PathBuf| path.strip_prefix(dir.path()).unwrap().to_path_buf();
    Tree {
        production: tree
            .production
            .into_iter()
            .map(|(path, module)| (relative(path), module))
            .collect(),
        test_only: tree.test_only.into_iter().map(relative).collect(),
    }
}

#[test]
fn walker_follows_inline_paths_includes_and_non_mod_rs_children() {
    let tree = walk_fixture(&[
        (
            "lib.rs",
            "#[path = \"d\"] mod m { mod c; }\nmod a;\ninclude!(\"spliced.rs\");\n\
             #[cfg(test)] #[path = \"t_tests.rs\"] mod t1;\n\
             #[cfg(test)] #[path = \"t_tests.rs\"] mod t2;\n\
             #[cfg(windows)] #[path = \"win.rs\"] mod imp;",
        ),
        ("d/c.rs", ""),
        ("t_tests.rs", ""),
        (
            "a.rs",
            "mod b;\n#[path = \"foo\"] mod inline { mod c2; }\nmod x { include!(\"inc.rs\"); }",
        ),
        ("a/b.rs", ""),
        ("a/foo/c2.rs", ""),
        ("inc.rs", ""),
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
    // Inline `#[path]` in a non-mod-rs file resolves under its stem
    // directory; `include!` against the file holding it.
    assert_eq!(
        module("a/foo/c2.rs"),
        Some(vec![
            "a".to_string(),
            "inline".to_string(),
            "c2".to_string()
        ])
    );
    assert_eq!(
        module("inc.rs"),
        Some(vec!["a".to_string(), "x".to_string()])
    );
    // Test-only mounts are not production and may share a file.
    assert_eq!(module("t_tests.rs"), None);
    assert!(
        walk_fixture_tree(&[
            ("lib.rs", "#[cfg(test)] mod fakes;"),
            ("fakes.rs", "mod inner;"),
            ("fakes/inner.rs", ""),
        ])
        .test_only
        .contains(Path::new("fakes/inner.rs"))
    );
    assert_eq!(module("s.rs"), Some(vec!["s".to_string()]));
}

#[test]
fn a_test_module_may_mount_a_production_file_in_either_order() {
    for lib in [
        "mod real;\n#[cfg(test)] #[path = \"real.rs\"] mod real_fixture;",
        "#[cfg(test)] #[path = \"real.rs\"] mod real_fixture;\nmod real;",
    ] {
        let tree = walk_fixture(&[("lib.rs", lib), ("real.rs", "")]);
        assert_eq!(
            tree.get(Path::new("real.rs")),
            Some(&vec!["real".to_string()]),
            "{lib}"
        );
    }
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
    let grouped: proc_macro2::TokenStream =
        "use crate::{application::secret::Bad, domain::{a::A, b as c}}; use crate as root; self as s"
            .parse()
            .unwrap();
    assert_eq!(
        crate_paths_in_tokens(grouped),
        [
            "crate::application::secret::Bad",
            "crate::domain::a::A",
            "crate::domain::b",
            UNRESOLVABLE,
            UNRESOLVABLE,
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
    let nested: proc_macro2::TokenStream = "serde(try_from = \"Vec<crate::application::X>\", \
         bound = \"T: crate::application::Tr\", rename = \"crate::not::code\")"
        .parse()
        .unwrap();
    assert_eq!(
        string_paths_in_tokens(nested),
        ["crate::application::X", "crate::application::Tr"]
    );
    let bound: proc_macro2::TokenStream =
        "serde(bound(serialize = \"T: crate::application::Tr\"), \
         rename(serialize = \"self-link\"))"
            .parse()
            .unwrap();
    assert_eq!(string_paths_in_tokens(bound), ["crate::application::Tr"]);
    let aliased: proc_macro2::TokenStream = "id! { use std as s; } extern crate dirs as d;"
        .parse()
        .unwrap();
    assert_eq!(crate_paths_in_tokens(aliased), [UNRESOLVABLE, UNRESOLVABLE]);
    let prose: proc_macro2::TokenStream =
        "error(\"crate::application::x failed\")".parse().unwrap();
    assert!(string_paths_in_tokens(prose).is_empty());
}

#[test]
#[should_panic(expected = "policy x: forbidden pattern crate::shell")]
fn assert_no_forbidden_names_the_rule_and_pattern() {
    assert_no_forbidden("policy x", "fn ok() {}", &["crate::shell"]);
    assert_no_forbidden("policy x", "use crate::shell::App;", &["crate::shell"]);
}
