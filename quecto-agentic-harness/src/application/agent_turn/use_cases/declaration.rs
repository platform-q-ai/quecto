//! Inactive declaration service seam. No acceptance or runtime wiring is implemented.
use std::sync::Mutex;

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

    pub fn snapshot(&self) -> Result<Option<TurnDeclaration>, DomainError> {
        self.state
            .lock()
            .map(|state| state.1.clone())
            .map_err(|_| DomainError::Other("declaration state unavailable".into()))
    }

    pub fn declare(
        &self,
        _authority: LaunchAuthority,
        _identity: &TurnIdentity,
        _intent: TurnIntent,
        _reason: &str,
    ) -> Result<TurnDeclaration, DomainError> {
        Err(DomainError::Tool(
            "turn declaration acceptance is not implemented".into(),
        ))
    }

    /// Future lifecycle entry point, deliberately not called by the runtime yet.
    pub fn enter(
        &self,
        _boundary: TurnBoundary,
        _identity: &TurnIdentity,
    ) -> Result<(), DomainError> {
        Err(DomainError::Tool(
            "turn declaration lifecycle is not implemented".into(),
        ))
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
