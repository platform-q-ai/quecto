//! Declaration-based test ownership shared by source conformance guards.

use std::path::Path;

// Match the established production_tokens::test_only AST policy exactly.
// Importing that whole module here would bring unrelated unused scan helpers.
fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Path>()
                .is_ok_and(|path| path.is_ident("test"))
    })
}

// As in dependency_scan, conditional path attributes count as possible owners.
fn path_of(meta: &syn::Meta) -> Option<String> {
    match meta {
        syn::Meta::NameValue(value) if value.path.is_ident("path") => match &value.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(path),
                ..
            }) => Some(path.value()),
            _ => None,
        },
        syn::Meta::List(list) if list.path.is_ident("cfg_attr") => {
            let nested = list
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                )
                .expect("cfg_attr has valid attributes");
            nested.iter().skip(1).find_map(path_of)
        }
        _ => None,
    }
}

/// Exempt only files whose every declaration proves test-only ownership.
/// Unknown owners, unresolved cycles and any production owner remain scanned.
pub(crate) fn declaration_test_sources(
    files: &[std::path::PathBuf],
) -> std::collections::BTreeSet<std::path::PathBuf> {
    use std::collections::{BTreeMap, BTreeSet};
    type Owners = BTreeMap<std::path::PathBuf, Vec<(std::path::PathBuf, bool)>>;
    fn declarations(
        items: &[syn::Item],
        owner: &Path,
        base: &Path,
        children: &Path,
        test: bool,
        owners: &mut Owners,
    ) {
        for item in items {
            if let syn::Item::Mod(module) = item {
                let test = test || test_only(&module.attrs);
                let name = module.ident.to_string();
                let explicit = module.attrs.iter().find_map(|attr| path_of(&attr.meta));
                if let Some((_, items)) = &module.content {
                    let dir = children.join(explicit.as_deref().unwrap_or(&name));
                    declarations(items, owner, &dir, &dir, test, owners);
                } else {
                    let candidates = match explicit {
                        Some(path) => vec![base.join(path)],
                        None => vec![
                            children.join(format!("{name}.rs")),
                            children.join(&name).join("mod.rs"),
                        ],
                    };
                    for target in candidates.into_iter().filter(|path| path.is_file()) {
                        owners
                            .entry(target.canonicalize().unwrap())
                            .or_default()
                            .push((owner.to_path_buf(), test));
                    }
                }
            }
        }
    }
    let mut owners = Owners::new();
    for file in files
        .iter()
        .filter(|file| file.extension().is_some_and(|ext| ext == "rs"))
    {
        let file = file.canonicalize().unwrap();
        let source = std::fs::read_to_string(&file).unwrap();
        let parsed = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("parse {}: {error}", file.display()));
        let base = file.parent().unwrap();
        let children = if matches!(
            file.file_name().unwrap().to_str(),
            Some("mod.rs" | "lib.rs" | "main.rs")
        ) {
            base.to_path_buf()
        } else {
            base.join(file.file_stem().unwrap())
        };
        declarations(
            &parsed.items,
            &file,
            base,
            &children,
            test_only(&parsed.attrs),
            &mut owners,
        );
    }
    let mut proven = BTreeSet::new();
    loop {
        let before = proven.len();
        for (file, declarations) in &owners {
            if declarations
                .iter()
                .all(|(owner, test)| *test || proven.contains(owner))
            {
                proven.insert(file.clone());
            }
        }
        if proven.len() == before {
            return proven;
        }
    }
}
