use super::*;

#[tokio::test]
async fn neighbors_match_impl_identifiers_and_trait_edges_only() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(tmp.path().join("src/edges.rs"), "pub struct Foo;\npub struct NotFoo;\npub trait Build {}\nimpl Foo {}\nimpl NotFoo {}\nimpl Build for Foo {}\n").unwrap();
    let foo = call(&tool, r#"{"action":"neighbors","symbol":"Foo","limit":30}"#).await;
    let implementations = foo["implementations"].as_array().unwrap();
    assert_eq!(implementations.len(), 2, "{foo}");
    assert!(implementations.iter().all(|s| s["for_type"] == "Foo"));
    assert_eq!(foo["trait_relationships"].as_array().unwrap().len(), 1);
    assert_eq!(foo["trait_relationships"][0]["trait_name"], "Build");
    let trait_result = call(
        &tool,
        r#"{"action":"neighbors","symbol":"Build","limit":30}"#,
    )
    .await;
    assert_eq!(
        trait_result["trait_relationships"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(trait_result["trait_relationships"][0]["for_type"], "Foo");
}

#[tokio::test]
async fn neighbors_report_module_reexports_not_function_local_imports() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(tmp.path().join("src/lib.rs"), "pub struct Foo;\npub use std::fmt::Debug;\npub(crate) use std::fmt::Display;\nfn local() { use std::io::Read; }\nfn multiline() {\n    use std::io::Write;\n}\n").unwrap();
    let v = call(&tool, r#"{"action":"neighbors","symbol":"Foo","limit":30}"#).await;
    let imports = v["imports_uses"].as_array().unwrap();
    assert_eq!(imports.len(), 2, "{v}");
    assert!(imports.iter().any(|u| u["use_tree"] == "std::fmt::Debug"));
    assert!(imports.iter().any(|u| u["use_tree"] == "std::fmt::Display"));
}

#[tokio::test]
async fn qualified_item_and_impl_header_regressions() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(tmp.path().join("src/qualifiers.rs"), "pub const fn make() {}\nunsafe async fn boom() {}\nasync unsafe fn mixed() {}\nstruct Widget<T>(T);\ntrait Build {}\nimpl<T> Widget<T> where T: Clone { }\nimpl<T> Build for Widget<T>\nwhere T: Clone,\n{ }\n").unwrap();
    let make = call(&tool, r#"{"action":"find_symbol","symbol":"make"}"#).await;
    assert_eq!(make["matches"][0]["kind"], "fn");
    let funcs = call(
        &tool,
        r#"{"action":"query","query":"async_functions","limit":30}"#,
    )
    .await;
    assert!(
        funcs["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "boom"),
        "{funcs}"
    );
    assert!(
        funcs["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "mixed"),
        "{funcs}"
    );
    let impls = call(
        &tool,
        r#"{"action":"query","query":"trait_impls","limit":30}"#,
    )
    .await;
    assert!(
        impls["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["trait_name"] == "Build" && s["for_type"] == "Widget<T>"),
        "{impls}"
    );
}

#[tokio::test]
async fn neighbors_children_are_exact_module_declarations() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(
        tmp.path().join("src/lib.rs"),
        "mod outer { mod inner { fn work() {} } }\nfn inner() {}\nstruct outer;\n",
    )
    .unwrap();
    // Ambiguous name: select mod by stable id rather than bare name.
    let found = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"outer","limit":30}"#,
    )
    .await;
    let mod_id = found["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "mod")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let outer = call(
        &tool,
        &serde_json::json!({"action":"neighbors","symbol":mod_id,"limit":30}).to_string(),
    )
    .await;
    assert!(
        outer["child_declarations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "inner" && s["kind"] == "mod"),
        "{outer}"
    );
    let func_id = found["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "struct")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let non_mod = call(
        &tool,
        &serde_json::json!({"action":"neighbors","symbol":func_id,"limit":30}).to_string(),
    )
    .await;
    assert!(
        non_mod["child_declarations"].as_array().unwrap().is_empty(),
        "{non_mod}"
    );
}

#[tokio::test]
async fn neighbors_do_not_mix_equally_named_modules_in_workspace_crates() {
    let (tool, tmp) = tool_with_workspace();
    for (crate_name, child) in [("one", "One"), ("two", "Two")] {
        let root = tmp.path().join(crate_name);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            format!("[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\n"),
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub mod shared;\n").unwrap();
        std::fs::write(root.join("src/shared.rs"), format!("pub struct {child};\n")).unwrap();
    }
    let found = call(
        &tool,
        r#"{"action":"find_symbol","symbol":"shared","limit":30}"#,
    )
    .await;
    let id = found["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["location"]["file"] == "one/src/lib.rs")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let v = call(
        &tool,
        &serde_json::json!({"action":"neighbors","symbol":id,"limit":30}).to_string(),
    )
    .await;
    assert!(
        v["child_declarations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "One"),
        "{v}"
    );
    assert!(
        !v["child_declarations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "Two"),
        "{v}"
    );
}

#[tokio::test]
async fn neighbors_inline_module_imports_are_scoped_to_module() {
    let (tool, tmp) = tool_with_workspace();
    std::fs::write(tmp.path().join("src/lib.rs"), "mod outer {\n    use std::fmt::Debug;\n    pub struct Foo;\n    fn local() { use std::io::Read; }\n}\nuse std::fmt::Display;\n").unwrap();
    let v = call(&tool, r#"{"action":"neighbors","symbol":"Foo","limit":30}"#).await;
    let imports = v["imports_uses"].as_array().unwrap();
    assert_eq!(imports.len(), 1, "{v}");
    assert_eq!(imports[0]["use_tree"], "std::fmt::Debug");
}

#[tokio::test]
async fn neighbors_external_module_children_match_exact_path() {
    let (tool, _tmp) = tool_with_workspace();
    let v = call(
        &tool,
        r#"{"action":"neighbors","symbol":"nested","limit":30}"#,
    )
    .await;
    assert!(
        v["child_declarations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "Worker"),
        "{v}"
    );
}
