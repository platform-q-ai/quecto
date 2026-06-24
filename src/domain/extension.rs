//! Extension trait: a modular unit that contributes tools and optional
//! system prompt context to the agent.

use std::sync::Arc;

use super::tool::Tool;

/// An extension contributes tools and optional system prompt context.
///
/// Extensions are the composition unit for adding capabilities to the agent.
/// Native (compiled-in) extensions and UDS-registered extensions both implement
/// this trait.
pub trait Extension: Send + Sync {
    /// Unique name for this extension.
    fn name(&self) -> &str;

    /// Human-readable description of this extension.
    fn description(&self) -> &str {
        ""
    }

    /// Tools this extension provides.
    fn tools(&self) -> Vec<Arc<dyn Tool>>;

    /// Optional text injected into the system prompt each turn.
    /// Called fresh each time so stateful extensions can reflect current state.
    fn system_prompt_snippet(&self) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
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
}
