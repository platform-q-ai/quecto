//! The explicit resume actions this harness can execute (#2011): the one
//! place an executor's slice declares its action available. Until then a
//! decision offers the action as unavailable, with the reason, and Cancel.
use crate::application::sessions::dto::ResumeActionCapabilities;

/// No executor is composed yet. #2012 (`OpenOriginal`), #2013 (`ForkCurrent`)
/// and #2014 (`Locate`, `Associate`) add `.with(ResumeAction::…)` here together
/// with the action's dispatch route: a flip alone fails the route test.
pub fn composed() -> ResumeActionCapabilities {
    ResumeActionCapabilities::cancel_only()
}

#[cfg(test)]
#[path = "resume_capabilities_tests.rs"]
mod tests;
