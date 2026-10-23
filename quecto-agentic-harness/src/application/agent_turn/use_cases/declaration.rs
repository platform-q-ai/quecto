//! Atomic declaration/lifecycle service; runtime composition remains inactive until C4.
use std::sync::{Mutex, MutexGuard};

use crate::domain::shared::error::DomainError;
use crate::domain::turn_declaration::{
    LaunchAuthority, TurnBoundary, TurnDeclaration, TurnIdentity, TurnIntent,
};

pub struct TurnDeclarationService {
    state: Mutex<(TurnIdentity, Option<TurnDeclaration>)>,
}

impl TurnDeclarationService {
    pub fn new(identity: TurnIdentity) -> Self {
        Self {
            state: Mutex::new((identity, None)),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, (TurnIdentity, Option<TurnDeclaration>)>, DomainError> {
        self.state
            .lock()
            .map_err(|_| DomainError::Other("declaration state unavailable".into()))
    }

    pub fn snapshot(&self) -> Result<Option<TurnDeclaration>, DomainError> {
        Ok(self.lock()?.1.clone())
    }

    pub fn declare(
        &self,
        authority: LaunchAuthority,
        identity: &TurnIdentity,
        intent: TurnIntent,
        reason: &str,
    ) -> Result<TurnDeclaration, DomainError> {
        authority.admit()?;
        let mut state = self.lock()?;
        if state.0 == *identity {
            let receipt = TurnDeclaration::validated(identity.clone(), intent, reason)?;
            state.1 = Some(receipt.clone());
            debug_assert_eq!(state.0, receipt.identity);
            Ok(receipt)
        } else {
            Err(DomainError::Tool(
                crate::domain::turn_declaration::STALE.into(),
            ))
        }
    }

    /// Trusted lifecycle port: caller invokes OuterRun only after admission, never on retry.
    pub fn enter(
        &self,
        boundary: TurnBoundary,
        identity: &TurnIdentity,
    ) -> Result<(), DomainError> {
        let mut state = self.lock()?;
        match boundary {
            TurnBoundary::OuterRun => {
                *state = (identity.clone(), None);
                Ok(())
            }
            TurnBoundary::ProviderRetry | TurnBoundary::ToolIteration if state.0 == *identity => {
                Ok(())
            }
            TurnBoundary::ProviderRetry | TurnBoundary::ToolIteration => Err(DomainError::Tool(
                crate::domain::turn_declaration::STALE.into(),
            )),
        }
    }
}

impl crate::application::agent_turn::ports::DeclareTurnOutcome for TurnDeclarationService {
    fn declare(
        &self,
        authority: LaunchAuthority,
        identity: &TurnIdentity,
        intent: TurnIntent,
        reason: &str,
    ) -> Result<TurnDeclaration, DomainError> {
        self.declare(authority, identity, intent, reason)
    }
}

#[cfg(test)]
#[path = "declaration_tests.rs"]
mod issue_2708_declaration_tests;
