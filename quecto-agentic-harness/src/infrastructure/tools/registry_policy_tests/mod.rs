//! The registry's policy tests: the shared imports live here so both
//! child modules reach them through `use super::*`.
#![allow(unused_imports)]
use super::ToolRegistryImpl;
use super::tests::{DummyTestTool, test_registry};
use crate::application::tools::ports::{Tool, ToolPolicyMutator};
use crate::domain::tool::{
    ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus, ToolPolicyRequest,
    ToolProfileContext,
};
use crate::domain::tool_descriptor::{ProfileAvailabilityScope, ToolRestrictionReason};
use std::sync::Arc;

mod persisted;
#[path = "../registry_policy_tests.rs"]
mod policy;
