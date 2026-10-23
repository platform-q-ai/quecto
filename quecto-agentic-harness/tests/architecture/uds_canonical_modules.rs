//! Resolve the migrated UDS module graph using Rust's canonical filenames.
//! All syntactic declarations (even inactive cfg) require checked-in readable
//! canonical files; generated/optional absent modules and item include! are unsupported.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn redirects_path(meta: &syn::Meta) -> bool {
    if meta.path().is_ident("path") {
        true
    } else if meta.path().is_ident("cfg_attr") {
        if let syn::Meta::List(list) = meta {
            use syn::parse::Parser;
            let parser = syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated;
            return match parser.parse2(list.tokens.clone()) {
                Ok(arguments) => arguments.iter().skip(1).any(redirects_path),
                Err(_) => true,
            };
        }
        true
    } else {
        false
    }
}

// Inspect only explicit syntax: no import alias resolution or macro expansion.
fn imports_builtin_include(tree: &syn::UseTree, prefix: &[String]) -> bool {
    match tree {
        syn::UseTree::Path(path) => {
            let mut nested = prefix.to_vec();
            nested.push(path.ident.to_string());
            imports_builtin_include(&path.tree, &nested)
        }
        syn::UseTree::Group(group) => group
            .items
            .iter()
            .any(|tree| imports_builtin_include(tree, prefix)),
        syn::UseTree::Name(name) => builtin_include_leaf(prefix, &name.ident),
        syn::UseTree::Rename(rename) => builtin_include_leaf(prefix, &rename.ident),
        syn::UseTree::Glob(_) => false,
    }
}

fn builtin_include_leaf(prefix: &[String], leaf: &syn::Ident) -> bool {
    match prefix {
        [root] if matches!(root.as_str(), "std" | "core") => leaf == "include",
        [root, item] if matches!(root.as_str(), "std" | "core") && item == "include" => {
            leaf == "self"
        }
        _ => false,
    }
}

struct Resolver<F> {
    read: F,
    visited: BTreeSet<PathBuf>,
    violations: Vec<String>,
}
impl<F: Fn(&Path) -> Result<String, String>> Resolver<F> {
    fn file(&mut self, path: &Path) {
        if self.visited.insert(path.to_owned()) {
            match (self.read)(path)
                .and_then(|text| syn::parse_file(&text).map_err(|error| error.to_string()))
            {
                Ok(file) => {
                    let parent = path.parent().expect("module file has parent");
                    let directory = if path.file_name().is_some_and(|name| name == "mod.rs") {
                        parent.to_owned()
                    } else {
                        parent.join(path.file_stem().expect("Rust file has stem"))
                    };
                    self.items(&file.items, path, &directory);
                }
                Err(error) => self.violations.push(format!("{}: {error}", path.display())),
            }
        }
    }

    fn items(&mut self, items: &[syn::Item], owner: &Path, directory: &Path) {
        for item in items {
            if let syn::Item::Use(import) = item {
                if imports_builtin_include(&import.tree, &[]) {
                    self.violations.push(format!(
                        "{}: builtin include import is unsupported in the owned canonical module graph; declare modules explicitly",
                        owner.display()
                    ));
                }
            }
            if let syn::Item::Macro(item_macro) = item {
                if item_macro
                    .mac
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "include")
                {
                    self.violations.push(format!(
                        "{}: item include! is unsupported in the owned canonical module graph; declare modules explicitly",
                        owner.display()
                    ));
                }
            }
            if let syn::Item::Mod(module) = item {
                let name = module.ident.to_string();
                let canonical = [
                    directory.join(format!("{name}.rs")),
                    directory.join(&name).join("mod.rs"),
                ];
                let mut redirected = None;
                for attribute in &module.attrs {
                    if redirects_path(&attribute.meta) {
                        self.violations.push(format!(
                            "{}: module {name} redirects canonical resolution",
                            owner.display()
                        ));
                        if let syn::Meta::NameValue(value) = &attribute.meta {
                            if let syn::Expr::Lit(literal) = &value.value {
                                if let syn::Lit::Str(path) = &literal.lit {
                                    redirected = Some(
                                        owner.parent().expect("owner parent").join(path.value()),
                                    );
                                }
                            }
                        }
                    }
                }
                if let Some((_, children)) = &module.content {
                    self.items(children, owner, &directory.join(&name));
                } else {
                    let candidates: Vec<_> = canonical
                        .iter()
                        .filter(|path| (self.read)(path).is_ok())
                        .collect();
                    if candidates.len() == 1 && redirected.is_none() {
                        self.file(candidates[0]);
                    } else if let Some(target) = redirected {
                        // Continue the actual baseline graph to report every redirect.
                        self.file(&target);
                    } else {
                        self.violations.push(format!(
                            "{}: module {name} has {} readable canonical files (expected one); all syntactic declarations, including inactive cfg, require one checked-in readable canonical file; generated/optional absent modules are unsupported",
                            owner.display(),
                            candidates.len()
                        ));
                    }
                }
            }
        }
    }
}

fn inspect<F: Fn(&Path) -> Result<String, String>>(roots: &[PathBuf], read: F) -> Vec<String> {
    let mut resolver = Resolver {
        read,
        visited: BTreeSet::new(),
        violations: Vec::new(),
    };
    for root in roots {
        resolver.file(root);
    }
    resolver.violations.sort();
    resolver.violations
}

fn inspect_owned<F: Fn(&Path) -> Result<String, String>>(root: &Path, read: F) -> Vec<String> {
    let mut resolver = Resolver {
        read,
        visited: BTreeSet::new(),
        violations: Vec::new(),
    };
    let owner = root.join("mod.rs");
    match (resolver.read)(&owner)
        .and_then(|source| syn::parse_file(&source).map_err(|error| error.to_string()))
    {
        Ok(file) => {
            let roles = ["controllers", "framing", "wire"];
            let items: Vec<_> = file.items.into_iter().filter(|item| {
                matches!(item, syn::Item::Mod(module) if roles.iter().any(|role| module.ident == role))
            }).collect();
            for role in roles {
                let count = items
                    .iter()
                    .filter(|item| matches!(item, syn::Item::Mod(module) if module.ident == role))
                    .count();
                if count != 1 {
                    resolver.violations.push(format!(
                        "{}: role {role} has {count} declarations (expected one)",
                        owner.display()
                    ));
                }
            }
            resolver.items(&items, &owner, root);
        }
        Err(error) => resolver
            .violations
            .push(format!("{}: {error}", owner.display())),
    }
    resolver.file(
        &root
            .parent()
            .expect("interface directory")
            .join("cli/uds_tool_intercept.rs"),
    );
    resolver.violations.sort();
    resolver.violations
}

#[test]
fn uds_framing_wire_and_cli_interception_use_canonical_module_paths() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/interface/uds");
    let violations = inspect_owned(&root, |path| {
        std::fs::read_to_string(path).map_err(|error| error.to_string())
    });
    assert!(
        violations.is_empty(),
        "noncanonical UDS module graph:\n{}",
        violations.join("\n")
    );
}

fn owned_fixture(owner: &str, controller: &str) -> Vec<String> {
    let files: BTreeMap<_, _> = [
        ("interface/uds/mod.rs", owner),
        ("interface/uds/framing/mod.rs", ""),
        ("interface/uds/wire/mod.rs", ""),
        ("interface/cli/uds_tool_intercept.rs", ""),
        ("interface/uds/controllers/mod.rs", controller),
        ("interface/uds/controllers/actual.rs", ""),
        ("interface/uds/controllers/alias.rs", ""),
        ("interface/uds/alias.rs", ""),
    ]
    .into_iter()
    .map(|(path, text)| (PathBuf::from(path), text.to_owned()))
    .collect();
    inspect_owned(Path::new("interface/uds"), |path| {
        files
            .get(path)
            .cloned()
            .ok_or_else(|| "unreadable fixture".into())
    })
}

#[test]
fn ownership_accepts_canonical_roles_without_expanding_unowned_capabilities() {
    assert!(
        owned_fixture(
            "mod framing; mod wire; mod controllers; #[path = \"legacy.rs\"] mod sessions;",
            "mod actual;",
        )
        .is_empty()
    );
}

#[test]
fn ownership_rejects_controller_redirect_with_both_files_present() {
    let violations = owned_fixture(
        "mod framing; mod wire; mod controllers;",
        "#[path = \"alias.rs\"] mod actual;",
    );
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(violations[0].contains("redirects canonical resolution"));
}

#[test]
fn ownership_rejects_direct_role_owner_redirect_with_both_files_present() {
    let violations = owned_fixture(
        "mod framing; mod wire; #[path = \"alias.rs\"] mod controllers;",
        "",
    );
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(violations[0].contains("redirects canonical resolution"));
}

#[test]
fn ownership_rejects_nested_conditional_role_owner_redirect_with_both_files_present() {
    let violations = owned_fixture(
        "mod framing; mod wire; #[cfg_attr(unix, cfg_attr(test, path = \"alias.rs\"))] mod controllers;",
        "",
    );
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(violations[0].contains("redirects canonical resolution"));
}

fn fixture(files: &[(&str, &str)]) -> Vec<String> {
    let files: BTreeMap<_, _> = files
        .iter()
        .map(|(path, text)| (PathBuf::from(path), text.to_string()))
        .collect();
    inspect(&[PathBuf::from("slice/mod.rs")], |path| {
        files
            .get(path)
            .cloned()
            .ok_or_else(|| "unreadable fixture".into())
    })
}

#[test]
fn resolver_accepts_both_canonical_shapes_and_inline_test_modules() {
    assert!(
        fixture(&[
            (
                "slice/mod.rs",
                "mod leaf; mod folder; mod inline { #[cfg(test)] mod tests; }"
            ),
            ("slice/leaf.rs", "mod child; // #[path = \"ignored.rs\"]"),
            ("slice/leaf/child.rs", ""),
            ("slice/folder/mod.rs", "mod child;"),
            ("slice/folder/child.rs", ""),
            (
                "slice/inline/tests.rs",
                "const TEXT: &str = \"#[path = x]\";"
            ),
        ])
        .is_empty()
    );
}

#[test]
fn resolver_reports_missing_ambiguous_and_unreadable_roots() {
    assert_eq!(fixture(&[("slice/mod.rs", "mod missing;")]).len(), 1);
    assert_eq!(
        fixture(&[
            ("slice/mod.rs", "mod duplicate;"),
            ("slice/duplicate.rs", ""),
            ("slice/duplicate/mod.rs", "")
        ])
        .len(),
        1
    );
    assert_eq!(fixture(&[]).len(), 1);
    assert_eq!(fixture(&[("slice/mod.rs", "not rust !!")]).len(), 1);
}

#[test]
fn resolver_reports_redirects_even_when_canonical_and_follows_children() {
    let violations = fixture(&[
        (
            "slice/mod.rs",
            "#[path = \"alias.rs\"] mod actual; #[cfg(test)] #[path = \"tests.rs\"] mod tests;",
        ),
        ("slice/alias.rs", "#[path = \"nested.rs\"] mod nested;"),
        ("slice/nested.rs", ""),
        ("slice/tests.rs", ""),
    ]);
    assert_eq!(violations.len(), 3, "{violations:?}");
    assert!(
        violations
            .iter()
            .all(|message| message.contains("redirects canonical resolution"))
    );
}

fn assert_cfg_attr_redirect(attribute: &str) {
    let source = format!("{attribute} mod actual;");
    let violations = fixture(&[
        ("slice/mod.rs", &source),
        ("slice/actual.rs", ""),
        ("slice/alias.rs", ""),
    ]);
    assert_eq!(violations.len(), 1, "{attribute}: {violations:?}");
    assert!(violations[0].contains("redirects canonical resolution"));
}

#[test]
fn resolver_rejects_direct_cfg_attr_redirect() {
    assert_cfg_attr_redirect("#[cfg_attr(unix, path = \"alias.rs\")]");
}

#[test]
fn resolver_rejects_nested_cfg_attr_redirect() {
    assert_cfg_attr_redirect("#[cfg_attr(unix, cfg_attr(test, path = \"alias.rs\"))]");
}

// The legacy policy is retained outside this slice's owned controller graph.
fn controller_scan_includes<F: Fn(&str) -> bool>(file: &str, legacy: bool, production: F) -> bool {
    let path = Path::new(file);
    let relative = path
        .strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap_or(path);
    if relative.starts_with("src/interface/uds/controllers") {
        production(relative.to_str().expect("UTF-8 source filename"))
    } else {
        legacy
    }
}

pub(super) fn production_scan_file(file: &str, legacy: bool) -> bool {
    controller_scan_includes(file, legacy, |relative| {
        static VERIFIED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        VERIFIED.get_or_init(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/interface/uds");
            let violations = inspect_owned(&root, |path| {
                std::fs::read_to_string(path).map_err(|error| error.to_string())
            });
            assert!(
                violations.is_empty(),
                "invalid owned module graph: {violations:?}"
            );
        });
        if super::dependency_scan::production_file(relative) {
            true
        } else {
            assert!(
                super::dependency_scan::test_only_file(relative),
                "unclassified source: {relative}"
            );
            false
        }
    })
}

#[test]
fn canonical_controller_scan_excludes_only_ast_test_nodes() {
    let files = [
        ("lib.rs", "mod interface;"),
        ("interface/mod.rs", "mod uds;"),
        ("interface/uds/mod.rs", "mod controllers;"),
        (
            "interface/uds/controllers/mod.rs",
            "mod production; #[cfg(test)] mod tests;",
        ),
        (
            "interface/uds/controllers/production.rs",
            "fn production_boundary() {}",
        ),
        (
            "interface/uds/controllers/tests.rs",
            "mod nested; fn test_boundary() {}",
        ),
        (
            "interface/uds/controllers/tests/nested.rs",
            "fn nested_test_boundary() {}",
        ),
    ];
    let mounted =
        super::dependency_scan::fixture_mounted_under(&files, &["interface", "uds", "controllers"]);
    let production: BTreeSet<_> = mounted
        .iter()
        .filter(|(_, _, test)| !test)
        .map(|(path, _, _)| format!("src/{}", path.display()))
        .collect();
    let selected: BTreeSet<_> = mounted
        .iter()
        .map(|(path, _, _)| format!("src/{}", path.display()))
        .filter(|path| {
            controller_scan_includes(path, !path.ends_with("_tests.rs"), |file| {
                production.contains(file)
            })
        })
        .collect();
    assert_eq!(selected, production);
    assert_eq!(
        production.len(),
        2,
        "both production owner and leaf remain guarded"
    );
    for legacy in [false, true] {
        assert_eq!(
            controller_scan_includes("src/interface/cli/unrelated.rs", legacy, |_| !legacy),
            legacy
        );
    }
}

#[test]
fn canonical_controller_scan_inventory_preserves_every_production_node() {
    let mounted = super::dependency_scan::mounted_under(&["interface", "uds", "controllers"]);
    let mut selected = 0;
    let mut removed = Vec::new();
    let mut shared_removed = 0;
    for (path, _, test_only) in mounted {
        let file = path.to_str().expect("source filename");
        let shared_legacy = !file.ends_with("_tests.rs");
        let legacy = shared_legacy && !file.contains("/tests/");
        let production = super::dependency_scan::production_file(file);
        assert_eq!(production, !test_only, "AST classification: {file}");
        for policy in [legacy, shared_legacy] {
            assert_eq!(
                production_scan_file(file, policy),
                production,
                "scan classification: {file}"
            );
        }
        assert_eq!(
            production_scan_file(&format!("{}/{file}", env!("CARGO_MANIFEST_DIR")), legacy),
            production
        );
        eprintln!(
            "coverage {file}: shared_before={shared_legacy} teardown_before={legacy} after={production} AST_test_only={test_only}"
        );
        if production {
            selected += 1;
        }
        if shared_legacy && test_only {
            shared_removed += 1;
        }
        if legacy && test_only {
            removed.push(file.to_owned());
        }
    }
    let mut files = Vec::new();
    super::teardown_authority::walk(Path::new("src"), &mut files);
    let mut unchanged = 0;
    for file in files {
        if Path::new(&file).starts_with("src/interface/uds/controllers") {
            continue;
        }
        for legacy in [false, true] {
            assert_eq!(
                production_scan_file(&file, legacy),
                legacy,
                "unrelated policy changed: {file}"
            );
        }
        unchanged += 1;
    }
    eprintln!(
        "controller AST production nodes: {selected}; teardown false-production removals: {}; shared removals: {shared_removed}; unrelated files unchanged: {unchanged}",
        removed.len()
    );
    assert_eq!(selected, 34);
    assert_eq!(removed.len(), 26);
}

#[test]
fn ownership_rejects_item_includes_in_files_and_inline_modules() {
    for source in [
        "include!(\"included.rs\");",
        "include!(concat!(env!(\"OUT_DIR\"), \"/included.rs\"));",
        "mod inline { include!(\"included.rs\"); }",
        "mod inline { include!(concat!(\"included\", \".rs\")); }",
    ] {
        let violations = owned_fixture("mod framing; mod wire; mod controllers;", source);
        assert_eq!(violations.len(), 1, "{source}: {violations:?}");
        assert_eq!(
            violations[0],
            "interface/uds/controllers/mod.rs: item include! is unsupported in the owned canonical module graph; declare modules explicitly"
        );
    }
}

fn assert_qualified_item_includes_rejected(namespace: &str) {
    for source in [
        format!("{namespace}::include!(\"included.rs\");"),
        format!("::{namespace}::include!(\"included.rs\");"),
        format!("mod inline {{ {namespace}::include!(\"included.rs\"); }}"),
        format!("#[cfg(any())] {namespace}::include!(concat!(\"included\", \".rs\"));"),
    ] {
        let violations = owned_fixture("mod framing; mod wire; mod controllers;", &source);
        assert_eq!(violations.len(), 1, "{source}: {violations:?}");
        assert_eq!(
            violations[0],
            "interface/uds/controllers/mod.rs: item include! is unsupported in the owned canonical module graph; declare modules explicitly"
        );
    }
}

#[test]
fn ownership_rejects_std_qualified_item_includes() {
    assert_qualified_item_includes_rejected("std");
}

#[test]
fn ownership_rejects_core_qualified_item_includes() {
    assert_qualified_item_includes_rejected("core");
}

#[test]
fn ownership_accepts_near_named_item_macros() {
    for namespace in ["", "std::", "core::", "::std::", "::core::"] {
        for name in [
            "include_str",
            "include_bytes",
            "include_extra",
            "not_include",
        ] {
            let source = format!("{namespace}{name}!(\"included.rs\");");
            let violations = owned_fixture("mod framing; mod wire; mod controllers;", &source);
            assert!(violations.is_empty(), "{source}: {violations:?}");
        }
    }
}

#[test]
fn ownership_keeps_includes_in_unowned_modules_out_of_scope() {
    assert!(
        owned_fixture(
            "mod framing; mod wire; mod controllers; mod sessions { include!(\"legacy.rs\"); }",
            "const TEXT: &str = \"include!(ignored)\";",
        )
        .is_empty()
    );
}

#[test]
fn resolver_requires_materialized_files_even_for_inactive_cfg_modules() {
    let violations = fixture(&[("slice/mod.rs", "#[cfg(any())] mod optional;")]);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(violations[0].contains("all syntactic declarations, including inactive cfg, require one checked-in readable canonical file"), "{violations:?}");
    assert!(
        fixture(&[
            ("slice/mod.rs", "#[cfg(any())] mod optional;"),
            ("slice/optional.rs", ""),
        ])
        .is_empty()
    );
}

fn builtin_include_imports(root: &str) -> Vec<String> {
    [
        format!("use ::{root}::include as generated; generated!(\"included.rs\");"),
        format!("use {root}::include;"),
        format!("use {root}::{{include}};"),
        format!("use {root}::{{include as generated}}; generated!(\"included.rs\");"),
        format!("use ::{root}::{{{{include as generated}}}};"),
        format!("use {{{root}::include as generated}};"),
        format!("use {{{{{root}::{{include as generated}}}}}};"),
        format!("use {root}::include::self;"),
        format!("use {root}::include::{{self}};"),
        format!("use {root}::{{include::{{self as generated}}}};"),
        format!("use ::{{{root}::{{include::{{{{self as generated}}}}}}}};"),
        format!("#[cfg(any())] use {root}::include as generated;"),
    ]
    .into()
}

fn assert_builtin_include_imports_rejected(root: &str) {
    for import in builtin_include_imports(root) {
        for source in [import.clone(), format!("mod inline {{ {import} }}")] {
            let violations = owned_fixture("mod framing; mod wire; mod controllers;", &source);
            assert_eq!(violations.len(), 1, "{source}: {violations:?}");
            assert_eq!(
                violations[0],
                "interface/uds/controllers/mod.rs: builtin include import is unsupported in the owned canonical module graph; declare modules explicitly",
                "{source}"
            );
        }
    }
}

#[test]
fn ownership_rejects_explicit_std_include_import_syntax_family() {
    assert_builtin_include_imports_rejected("std");
}

#[test]
fn ownership_rejects_explicit_core_include_import_syntax_family() {
    assert_builtin_include_imports_rejected("core");
}

#[test]
fn ownership_keeps_builtin_include_imports_in_unowned_modules_out_of_scope() {
    for root in ["std", "core"] {
        for import in builtin_include_imports(root) {
            let owner = format!(
                "mod framing; mod wire; mod controllers; {import} mod sessions {{ {import} }}"
            );
            let violations = owned_fixture(&owner, "");
            assert!(violations.is_empty(), "{owner}: {violations:?}");
        }
    }
}

#[test]
fn ownership_accepts_near_named_and_unrelated_imports() {
    for root in ["std", "core"] {
        for name in [
            "include_str",
            "include_bytes",
            "include_extra",
            "not_include",
        ] {
            for import in [
                format!("use {root}::{name} as generated;"),
                format!("use ::{{{root}::{{{{{name} as include}}}}}};"),
                format!("use {root}::{name}::{{self as generated}};"),
            ] {
                let source = format!("{import} mod inline {{ {import} }}");
                let violations = owned_fixture("mod framing; mod wire; mod controllers;", &source);
                assert!(violations.is_empty(), "{source}: {violations:?}");
            }
        }
    }
    for import in [
        "use other::include as generated;",
        "use other::{std::{include as generated}};",
        "use std::other::{include as generated};",
        "use std::{self as include};",
        "use core as include;",
        "use std::*;",
    ] {
        let source = format!("{import} mod inline {{ {import} }}");
        let violations = owned_fixture("mod framing; mod wire; mod controllers;", &source);
        assert!(violations.is_empty(), "{source}: {violations:?}");
    }
}
