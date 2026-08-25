use std::fs;
use std::path::Path;

fn production_source() -> String {
    fn collect(path: &Path, out: &mut String) {
        for entry in fs::read_dir(path).expect("read source directory") {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                collect(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with("_tests.rs")
            {
                out.push_str(&fs::read_to_string(path).expect("read source file"));
            }
        }
    }

    let mut source = String::new();
    collect(Path::new("src"), &mut source);
    source
}

#[test]
fn catalogue_and_provider_application_seams_have_production_ownership() {
    let source = production_source();

    for decorative_seam in [
        "ResolveCatalogueUseCase",
        "ComposeCatalogueRuntimeUseCase",
        "CatalogueRuntimeComposer",
        "ComposeProviderRuntimeUseCase",
        "ProviderRuntimeFactory",
        "RefreshCatalogueSourceUseCase",
    ] {
        assert!(
            !source.contains(decorative_seam),
            "decorative or test-only application seam remains in production: {decorative_seam}"
        );
    }

    assert!(source.contains("ResolveModelLimitsUseCase"));
    assert!(source.contains("CatalogueRuntimeSnapshot"));
}
