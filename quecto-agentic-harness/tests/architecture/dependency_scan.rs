//! Shared scanning for the layer rules (#1637). Substring rules scan a file's
//! production tokens (`tests/common/production_tokens.rs`); path rules resolve
//! `super::`/`self::` against the file's place in the crate's module tree, so a
//! relative import is checked as the crate path it names.

#[path = "../common/production_tokens.rs"]
mod production_tokens;
pub(super) use production_tokens::{
    assert_no_forbidden, forbidden_hit, item_test_only, production_text, test_only,
};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// A crate source file's module path, from the crate's real module tree
/// (walked from `src/lib.rs` through every `mod` declaration, following
/// `#[path]`): `src/infrastructure/tools/spawn.rs` →
/// `["infrastructure", "tools", "spawn"]`, and a `#[path]` child is named by
/// the module that declares it, not by where its file sits. A file outside the
/// tree is a caller bug.
pub(super) fn module_path(file: &str) -> Vec<String> {
    static TREE: OnceLock<BTreeMap<PathBuf, Vec<String>>> = OnceLock::new();
    let tree = TREE.get_or_init(|| {
        let mut tree = BTreeMap::new();
        walk_module(Path::new("src/lib.rs"), true, Vec::new(), &mut tree);
        tree
    });
    tree.get(&normalize(Path::new(file)))
        .cloned()
        .unwrap_or_else(|| panic!("{file} is not in the crate module tree"))
}

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
    if tree.contains_key(&key) {
        return;
    }
    let source = std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("read module file {}: {e}", file.display()));
    let parsed = syn::parse_file(&source)
        .unwrap_or_else(|e| panic!("parse module file {}: {e}", file.display()));
    tree.insert(key, module.clone());
    let file_dir = file.parent().expect("module file has a directory");
    let child_dir = if mod_rs {
        file_dir.to_path_buf()
    } else {
        file_dir.join(file.file_stem().expect("module file has a stem"))
    };
    walk_items(&parsed.items, file_dir, &child_dir, &module, tree);
}

fn walk_items(
    items: &[syn::Item],
    path_base: &Path,
    child_dir: &Path,
    module: &[String],
    tree: &mut BTreeMap<PathBuf, Vec<String>>,
) {
    for item in items {
        let syn::Item::Mod(declared) = item else {
            continue;
        };
        let name = declared.ident.to_string();
        let mut child = module.to_vec();
        child.push(name.clone());
        if let Some((_, inline)) = &declared.content {
            // `#[path]` inside an inline module is relative to the inline
            // module's own directory.
            let dir = child_dir.join(&name);
            walk_items(inline, &dir, &dir, &child, tree);
            continue;
        }
        let explicit = declared.attrs.iter().find_map(|attr| match &attr.meta {
            syn::Meta::NameValue(value) if value.path.is_ident("path") => match &value.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(path),
                    ..
                }) => Some(path.value()),
                _ => None,
            },
            _ => None,
        });
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
