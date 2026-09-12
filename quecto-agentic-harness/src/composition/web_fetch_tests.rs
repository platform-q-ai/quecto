use super::web_fetch::build_web_fetch_tool;

#[test]
fn factory_builds_stable_web_fetch_tool() {
    let tool = build_web_fetch_tool(32);
    assert_eq!(tool.definition().name.as_ref(), "web_fetch");
}
