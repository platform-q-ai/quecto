use super::*;
use crate::application::admission::dto::{NegotiationInputs, NegotiationPlan};
use std::path::PathBuf;

#[test]
fn a_child_context_wins_even_when_a_section_is_configured() {
    // #2023: a child with its own (possibly diverging or null) config still
    // inherits the parent's authority; the context always wins.
    let plan = NegotiateAuthority::new().execute(NegotiationInputs {
        configured_directory: Some(PathBuf::from("/x/admission")),
        inherited_context: Some(PathBuf::from("/tmp/ctx")),
    });
    assert_eq!(
        plan,
        NegotiationPlan::Child {
            context: PathBuf::from("/tmp/ctx")
        }
    );
}

#[test]
fn a_configured_root_registers_at_its_directory() {
    let plan = NegotiateAuthority::new().execute(NegotiationInputs {
        configured_directory: Some(PathBuf::from("/x/admission")),
        inherited_context: None,
    });
    assert_eq!(
        plan,
        NegotiationPlan::Root {
            directory: PathBuf::from("/x/admission")
        }
    );
}

#[test]
fn nothing_configured_is_disabled() {
    let plan = NegotiateAuthority::new().execute(NegotiationInputs {
        configured_directory: None,
        inherited_context: None,
    });
    assert_eq!(plan, NegotiationPlan::Disabled);
}
