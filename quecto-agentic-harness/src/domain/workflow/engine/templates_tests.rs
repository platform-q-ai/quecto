use super::*;

#[test]
fn built_in_default_templates_are_empty() {
    // Workflow templates are intentionally no longer embedded from the public
    // repo. Sessions load templates from configured/user-local directories or
    // inline config instead.
    assert!(default_templates().is_empty());
}
