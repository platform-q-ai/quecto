//! Decide how a starting process joins the authority (#2024 S3, #2023).
//!
//! This is the one home of the rule that a child inherits its parent's
//! authority unconditionally: an inherited `--admission-context` always wins,
//! whatever the child's own config says (which S1 made unable to carry an
//! `admission` section at all, and whose explicit `--config` may still differ
//! or be `null`). The byte-equal candidate check that used to reject a
//! diverging child is retired in favour of this rule plus the `inherit` flag
//! the composition passes to the runtime factory.
//!
//! The decision is pure: the interface observes the inputs (does a section
//! resolve to a directory, is a context present) and performs the resulting
//! plan through infrastructure.

use crate::application::admission::dto::{NegotiationInputs, NegotiationPlan};

#[derive(Debug, Default)]
pub struct NegotiateAuthority;

impl NegotiateAuthority {
    pub fn new() -> Self {
        Self
    }

    pub fn execute(&self, inputs: NegotiationInputs) -> NegotiationPlan {
        // A child wins unconditionally: an inherited context binds the
        // parent's capability regardless of the child's own configuration.
        if let Some(context) = inputs.inherited_context {
            return NegotiationPlan::Child { context };
        }
        match inputs.configured_directory {
            Some(directory) => NegotiationPlan::Root { directory },
            None => NegotiationPlan::Disabled,
        }
    }
}

#[cfg(test)]
#[path = "negotiate_authority_tests.rs"]
mod tests;
