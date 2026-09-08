//! Explicit, restart-only admission ingress. The normal runtime factory remains disabled.
//!
//! Alias bindings are operator-supplied router-slot identities, never derived from
//! credentials, endpoint URLs, model IDs, or refreshed OAuth account claims.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::provider_runtime::{AgentRuntimeInputs, compose_agent_provider_inner};
use crate::application::ports::AttemptAdmission;
use crate::application::ports::ProviderRuntimeFactory;
use crate::domain::inference_admission::{AdmissionConfig, GroupPolicy};
use crate::domain::provider::LlmProvider;
use crate::infrastructure::config::Config;

#[derive(Debug, Clone)]
pub struct AdmissionRuntimeProposal {
    pub policy: AdmissionConfig,
    /// Router provider slot -> explicit endpoint/account alias.
    pub bindings: BTreeMap<String, String>,
}

/// Stable authority capabilities, retained across credential/model rebuilds.
/// Construct a new context only at a process/runtime restart boundary.
pub struct AdmissionRuntimeContext {
    effective: AdmissionRuntimeProposal,
    gates_by_alias: BTreeMap<String, Arc<dyn AttemptAdmission>>,
}

impl AdmissionRuntimeContext {
    pub fn new(
        proposal: AdmissionRuntimeProposal,
        gates_by_alias: BTreeMap<String, Arc<dyn AttemptAdmission>>,
    ) -> Result<Self, String> {
        proposal
            .policy
            .validate()
            .map_err(|e| format!("invalid admission policy: {e:?}"))?;
        for (slot, alias) in &proposal.bindings {
            if slot.is_empty() || slot.trim() != slot || slot.contains('/') {
                return Err("invalid admission provider binding".into());
            }
            if !proposal.policy.aliases.contains_key(alias) || !gates_by_alias.contains_key(alias) {
                return Err(format!(
                    "admission binding for '{slot}' has an unknown or unbound alias"
                ));
            }
        }
        Ok(Self {
            effective: proposal,
            gates_by_alias,
        })
    }

    pub(crate) fn gate(&self, slot: &str) -> Result<Arc<dyn AttemptAdmission>, String> {
        let alias = self.effective.bindings.get(slot).ok_or_else(|| {
            format!("admission provider '{slot}' requires an explicit alias binding")
        })?;
        self.gates_by_alias
            .get(alias)
            .cloned()
            .ok_or_else(|| format!("admission provider '{slot}' has no bound capability"))
    }

    fn validate_candidate(
        &self,
        proposal: Option<&AdmissionRuntimeProposal>,
    ) -> Result<(), String> {
        let equal = proposal.is_some_and(|p| {
            p.bindings == self.effective.bindings && equal_policy(&p.policy, &self.effective.policy)
        });
        if equal {
            Ok(())
        } else {
            Err(
                "admission policy, aliases, bindings or enabled-state changed; restart required"
                    .into(),
            )
        }
    }
}

fn equal_group(a: &GroupPolicy, b: &GroupPolicy) -> bool {
    // Exhaustive destructuring makes a newly added policy field a compile-time review obligation.
    let GroupPolicy {
        capacity,
        reserve,
        min_interval_ms,
        queue_capacity,
        queue_timeout_ms,
        attempt_timeout_ms,
        max_cooldown_ms,
        fallback_base_ms,
    } = a;
    (
        *capacity,
        *reserve,
        *min_interval_ms,
        *queue_capacity,
        *queue_timeout_ms,
        *attempt_timeout_ms,
        *max_cooldown_ms,
        *fallback_base_ms,
    ) == (
        b.capacity,
        b.reserve,
        b.min_interval_ms,
        b.queue_capacity,
        b.queue_timeout_ms,
        b.attempt_timeout_ms,
        b.max_cooldown_ms,
        b.fallback_base_ms,
    )
}
fn equal_policy(a: &AdmissionConfig, b: &AdmissionConfig) -> bool {
    let AdmissionConfig {
        groups,
        aliases,
        max_scopes,
        terminal_capacity,
    } = a;
    aliases == &b.aliases
        && *max_scopes == b.max_scopes
        && *terminal_capacity == b.terminal_capacity
        && groups.len() == b.groups.len()
        && groups.iter().all(|(id, policy)| {
            b.groups
                .get(id)
                .is_some_and(|other| equal_group(policy, other))
        })
}

pub struct AdmissionRuntimeCandidate<'a> {
    pub providers: &'a Config,
    pub admission: Option<&'a AdmissionRuntimeProposal>,
}

pub struct AdmissionProviderRuntimeFactory {
    context: Arc<AdmissionRuntimeContext>,
}
impl AdmissionProviderRuntimeFactory {
    pub fn new(context: Arc<AdmissionRuntimeContext>) -> Self {
        Self { context }
    }
}
impl ProviderRuntimeFactory<AdmissionRuntimeCandidate<'_>, AgentRuntimeInputs>
    for AdmissionProviderRuntimeFactory
{
    fn compose_runtime(
        &self,
        candidate: &AdmissionRuntimeCandidate<'_>,
        inputs: &AgentRuntimeInputs,
    ) -> Result<Arc<dyn LlmProvider>, String> {
        compose_agent_provider_with_admission(
            candidate.providers,
            inputs,
            &self.context,
            candidate.admission,
        )
    }
}

/// Compose with an existing immutable authority. Rejections occur before publication;
/// the application's composition use case retains both live runtime and catalogue.
/// Unlike the disabled path, OpenAI OAuth rebuilds use a kernel-owned, Codex-aware
/// constructor so the admission-unaware `openai_oauth_factory` cannot lose the gate.
pub fn compose_agent_provider_with_admission(
    config: &Config,
    inputs: &AgentRuntimeInputs,
    context: &AdmissionRuntimeContext,
    proposal: Option<&AdmissionRuntimeProposal>,
) -> Result<Arc<dyn LlmProvider>, String> {
    context.validate_candidate(proposal)?;
    compose_agent_provider_inner(config, inputs, Some(context))
}

#[cfg(test)]
#[path = "provider_runtime_admission_tests.rs"]
mod tests;
