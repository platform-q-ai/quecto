//! Entry-point contract for provider runtime composition (issue #1573, epic
//! #1193 slice 3; #1849 PR 1): no provider construction, credential
//! resolution, or router orchestration remains anywhere under `interface/`.
//! That orchestration lives behind the application's `ProviderRuntimeFactory`
//! port (implemented in `infrastructure::providers::routing::provider_runtime`); the composition
//! layer (`composition::runtime`) wires dependencies and invokes the shared
//! compose use case, and entry points receive its builder by injection.
//!
//! Source-level ratchet: reintroducing a construction call into any
//! `interface` module fails this test before review ever sees it.

use std::path::Path;

/// Symbols that only infrastructure and composition may touch. Any of these
/// appearing in a non-test `interface` source means provider construction,
/// refresh wiring or router orchestration leaked back into the interface.
const FORBIDDEN: &[&str] = &[
    "ProviderRouter",
    "RetryingProvider",
    "RefreshableProvider",
    "create_openai_compatible_provider",
    "create_anthropic_compatible_provider",
    "create_provider_with_client",
    "create_named_openai_provider_with_client",
    "create_openai_provider_with_client",
    "create_codex_provider_with_client",
    "make_provider_factory",
    "make_oauth_refresh_fn",
    "compose_and_publish_runtime",
    "ComposeProviderRuntimeUseCase",
];

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
fn declaration_test_sources(
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

#[test]
fn interface_contains_no_provider_construction_or_router_orchestration() {
    let interface_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/interface");
    assert_interface_contains_no_provider_construction(&interface_dir);
}

fn assert_interface_contains_no_provider_construction(interface_dir: &Path) {
    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    let files = walk(interface_dir);
    let test_sources = declaration_test_sources(&files);
    for entry in files {
        let name = entry.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".rs") || test_sources.contains(&entry.canonicalize().unwrap()) {
            continue;
        }
        scanned += 1;
        let source = std::fs::read_to_string(&entry).unwrap();
        for symbol in FORBIDDEN {
            if source.contains(symbol) {
                offenders.push(format!("{name}: {symbol}"));
            }
        }
        // Credential resolution is confined to the composition layer: no
        // interface module opens the credential store, the `auth` command
        // included (its use cases reach the store through composed handles).
        if source.contains("CredentialStore::new") {
            offenders.push(format!("{name}: CredentialStore::new"));
        }
    }
    assert!(
        scanned > 50,
        "expected to scan the interface modules, saw {scanned}"
    );
    assert!(
        offenders.is_empty(),
        "provider construction leaked back into interface:\n{}",
        offenders.join("\n")
    );
}

/// Composition's entry point routes through the shared composition use case:
/// the provider it returns IS the published runtime generation's provider,
/// and the runtime + catalogue stores publish one coherent generation.
#[test]
fn build_agent_provider_publishes_through_the_composition_use_case() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"entryish":{"api":"openai-completions","apiKey":"sk-entry",
            "baseUrl":"https://api.example.test/v1",
            "models":[{"id":"entry-model","name":"Entry Model"}]}}}"#,
    )
    .unwrap();
    let config = quecto::infrastructure::config::Config::default();
    let provider = quecto::composition::runtime::build_agent_provider(
        &config,
        tmp.path(),
        &reqwest::Client::new(),
    )
    .expect("provider builds");
    let published =
        quecto::infrastructure::providers::catalogue::catalogue_registry::runtime_store_for(
            tmp.path(),
        )
        .current()
        .expect("the entry point published a runtime generation");
    assert!(
        std::sync::Arc::ptr_eq(&provider, &published.provider),
        "the returned provider must be the published runtime generation's provider"
    );
    assert_eq!(
        published.generation(),
        quecto::infrastructure::providers::catalogue::catalogue_registry::snapshot_store_for(
            tmp.path()
        )
        .current()
        .generation(),
        "runtime and catalogue stores share one generation"
    );
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

mod l13e_test_ownership {
    use super::assert_interface_contains_no_provider_construction;
    fn fixture(child: &str) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let controllers = root.path().join("cli/controllers");
        std::fs::create_dir_all(controllers.join("shared")).unwrap();
        // Mirror src/interface/cli/controllers/shared.rs and its ordinary child.
        std::fs::write(controllers.join("shared.rs"), "#[cfg(test)]\nmod tests;\n").unwrap();
        std::fs::write(controllers.join("shared/tests.rs"), child).unwrap();
        for index in 0..51 {
            std::fs::write(root.path().join(format!("module_{index}.rs")), "").unwrap();
        }
        root
    }

    const CREDENTIAL_TEST: &str = "use crate::infrastructure::auth::credential_store::CredentialStore;\n\
        #[test]\nfn credential_fixture() {\n\
        let tmp = tempfile::tempdir().unwrap();\n\
        let _store = CredentialStore::new(tmp.path());\n}\n";

    #[test]
    fn canonical_external_cfg_test_child_is_not_production() {
        let root = fixture(CREDENTIAL_TEST);
        assert_interface_contains_no_provider_construction(root.path());
    }

    #[test]
    #[should_panic(expected = "production.rs: CredentialStore::new")]
    fn production_credential_construction_is_rejected() {
        let root = fixture("");
        std::fs::write(
            root.path().join("production.rs"),
            "fn build() { let _store = CredentialStore::new(std::path::Path::new(\".\")); }",
        )
        .unwrap();
        assert_interface_contains_no_provider_construction(root.path());
    }

    #[test]
    #[should_panic(expected = "production.rs: create_openai_compatible_provider")]
    fn production_provider_construction_is_rejected() {
        let root = fixture("");
        std::fs::write(
            root.path().join("production.rs"),
            "fn build() { create_openai_compatible_provider(); }",
        )
        .unwrap();
        assert_interface_contains_no_provider_construction(root.path());
    }

    #[test]
    #[should_panic(expected = "production.rs: ProviderRouter")]
    fn production_router_orchestration_is_rejected() {
        let root = fixture("");
        std::fs::write(
            root.path().join("production.rs"),
            "use router::ProviderRouter;",
        )
        .unwrap();
        assert_interface_contains_no_provider_construction(root.path());
    }

    #[test]
    #[should_panic(expected = "tests.rs: CredentialStore::new")]
    fn child_owned_by_production_and_tests_is_rejected() {
        let root = fixture(CREDENTIAL_TEST);
        std::fs::write(
            root.path().join("cli/controllers/shared.rs"),
            "#[cfg(test)]\nmod tests;\n#[path = \"shared/tests.rs\"]\nmod production;\n",
        )
        .unwrap();
        assert_interface_contains_no_provider_construction(root.path());
    }
}
