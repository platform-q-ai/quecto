//! The explicit resume actions this harness can execute (#2011): the one
//! place an executor's slice declares its action available. Until then a
//! decision offers the action as unavailable, with the reason, and Cancel.
use crate::application::sessions::dto::ResumeActionCapabilities;

/// No executor is composed yet: #2012 adds `OpenOriginal`, #2013
/// `ForkCurrent`, #2014 `Locate` and `Associate` — each with
/// `.with(ResumeAction::…)` here, when it wires its use case.
pub fn composed() -> ResumeActionCapabilities {
    ResumeActionCapabilities::cancel_only()
}

#[cfg(test)]
#[path = "resume_capabilities_tests.rs"]
mod tests;
