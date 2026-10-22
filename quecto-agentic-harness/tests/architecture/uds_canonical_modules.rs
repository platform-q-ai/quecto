//! Resolve the migrated UDS module graph using Rust's canonical filenames.
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
                            "{}: module {name} has {} readable canonical files (expected one)",
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

#[test]
fn uds_framing_wire_and_cli_interception_use_canonical_module_paths() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/interface/uds");
    let violations = inspect(
        &[
            root.join("framing/mod.rs"),
            root.join("wire/mod.rs"),
            root.parent()
                .expect("interface directory")
                .join("cli/uds_tool_intercept.rs"),
        ],
        |path| std::fs::read_to_string(path).map_err(|error| error.to_string()),
    );
    assert!(
        violations.is_empty(),
        "noncanonical UDS module graph:\n{}",
        violations.join("\n")
    );
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
