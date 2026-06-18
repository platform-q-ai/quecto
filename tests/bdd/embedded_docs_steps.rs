use super::*;
use quecto::domain::tool::Tool;
use quecto::infrastructure::tools::docs::DocsTool;

async fn run_docs(world: &mut QuectoWorld, args: &str) {
    let result = DocsTool::new().execute(args).await.expect("docs tool");
    world.docs_output = result.content;
    world.docs_is_error = result.is_error;
}

#[when("I list the embedded docs")]
async fn when_list_embedded_docs(world: &mut QuectoWorld) {
    run_docs(world, "{}").await;
}

#[when(expr = "I read the embedded doc {string}")]
async fn when_read_embedded_doc(world: &mut QuectoWorld, name: String) {
    let args = serde_json::json!({ "name": name }).to_string();
    run_docs(world, &args).await;
}

#[then(expr = "the docs listing should include {string}")]
fn then_listing_includes(world: &mut QuectoWorld, name: String) {
    assert!(
        world.docs_output.contains(&name),
        "docs listing should include '{name}':\n{}",
        world.docs_output
    );
}

#[then(expr = "the embedded doc content should contain {string}")]
fn then_doc_contains(world: &mut QuectoWorld, needle: String) {
    assert!(
        world.docs_output.contains(&needle),
        "embedded doc should contain '{needle}'"
    );
}

#[then("reading the embedded doc should fail")]
fn then_reading_failed(world: &mut QuectoWorld) {
    assert!(world.docs_is_error, "expected the docs read to be an error");
}
