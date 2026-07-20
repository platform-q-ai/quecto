use super::*;

/// Minimal extension exercising the trait's default methods.
struct BareExtension;

impl Extension for BareExtension {
    fn name(&self) -> &str {
        "bare"
    }
    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![]
    }
}

#[test]
fn extension_defaults_are_empty() {
    let ext = BareExtension;
    assert_eq!(ext.name(), "bare");
    assert_eq!(ext.description(), "");
    assert!(ext.tools().is_empty());
    assert_eq!(ext.system_prompt_snippet(), None);
}
