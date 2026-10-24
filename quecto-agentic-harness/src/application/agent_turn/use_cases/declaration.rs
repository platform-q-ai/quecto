//! Pure declaration service; runtime admission and tool activation remain separate.
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
        self.lock().map(|state| state.1.clone())
    }

    pub fn declare(
        &self,
        authority: LaunchAuthority,
        identity: &TurnIdentity,
        intent: TurnIntent,
        reason: &str,
    ) -> Result<TurnDeclaration, DomainError> {
        match authority {
            LaunchAuthority::TopLevelParent => {}
            _ => {
                return Err(DomainError::Tool(
                    "Turn outcomes may be declared only by the active top-level parent turn."
                        .into(),
                ));
            }
        }
        let trimmed = reason.trim();
        let one_line = reason.split(['\r', '\n']).count() == 1;
        let bounded = (1..=512).contains(&trimmed.chars().count());
        let normalized = match (one_line, bounded) {
            (true, true) => trimmed.to_owned(),
            _ => return Err(DomainError::Tool(
                "Choose waiting, blocked, asking, or done and provide a nonempty one-line reason of at most 512 characters.".into(),
            )),
        };
        let mut state = self.lock()?;
        if &state.0 == identity {
            let receipt = TurnDeclaration {
                identity: identity.clone(),
                intent,
                reason: normalized,
            };
            debug_assert_eq!(&receipt.identity, &state.0);
            state.1 = Some(receipt.clone());
            Ok(receipt)
        } else {
            Err(Self::stale_identity())
        }
    }

    /// Called by the trusted owner after admitting an outer run. Internal attempts
    /// must present the current identity and retain the complete declaration.
    /// This service does not itself admit callers or wire runtime lifecycle events.
    pub fn enter(
        &self,
        boundary: TurnBoundary,
        identity: &TurnIdentity,
    ) -> Result<(), DomainError> {
        let mut state = self.lock()?;
        match boundary {
            TurnBoundary::OuterRun => {
                *state = (identity.clone(), None);
                debug_assert_eq!(&state.0, identity);
                Ok(())
            }
            TurnBoundary::ProviderRetry | TurnBoundary::ToolIteration => {
                if &state.0 == identity {
                    Ok(())
                } else {
                    Err(Self::stale_identity())
                }
            }
        }
    }

    fn stale_identity() -> DomainError {
        DomainError::Tool("This declaration does not belong to the active turn.".into())
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
// Preserve the original acceptance module byte-for-byte, including its boolean comparison.
#[allow(clippy::bool_comparison)]
#[path = "declaration_tests.rs"]
mod issue_2708_declaration_tests;
