//! An in-memory [`BoardTokens`] for tests (epic #2705 B5): tokens are
//! `token-<name>`, kept as given; [`MemoryBoardTokens::fail_with`] makes
//! every call fail.
use std::sync::Mutex;

use crate::application::project_board::ports::{
    BoardTokens, IssuedToken, MAX_TOKENS, TokenError, TokenKind, TokenRecord,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

#[derive(Default)]
pub struct MemoryBoardTokens {
    held: Mutex<Vec<(TokenRecord, String)>>,
    failing: Mutex<Option<TokenError>>,
}

impl MemoryBoardTokens {
    /// Every call from now on fails with `error`.
    pub fn fail_with(&self, error: TokenError) {
        *self.failing.lock().unwrap() = Some(error);
    }

    fn failing(&self) -> Result<(), TokenError> {
        self.failing.lock().unwrap().clone().map_or(Ok(()), Err)
    }
}

impl MemoryBoardTokens {
    fn added(
        &self,
        name: &Slug,
        created: &Timestamp,
        kind: TokenKind,
    ) -> Result<IssuedToken, TokenError> {
        self.failing()?;
        let mut held = self.held.lock().unwrap();
        if held.iter().any(|(record, _)| &record.name == name) {
            return Err(TokenError::Exists(name.clone()));
        }
        if held.len() >= MAX_TOKENS {
            return Err(TokenError::Full);
        }
        let record = TokenRecord {
            name: name.clone(),
            created: created.clone(),
            kind,
        };
        let secret = format!("token-{}", name.as_str());
        held.push((record.clone(), secret.clone()));
        Ok(IssuedToken { record, secret })
    }
}

impl BoardTokens for MemoryBoardTokens {
    fn add(&self, name: &Slug, created: &Timestamp) -> Result<IssuedToken, TokenError> {
        self.added(name, created, TokenKind::Device)
    }

    fn add_replica(&self, name: &Slug, created: &Timestamp) -> Result<IssuedToken, TokenError> {
        self.added(name, created, TokenKind::Replica)
    }

    fn kind_of(&self, name: &Slug) -> Result<Option<TokenKind>, TokenError> {
        self.failing()?;
        let held = self.held.lock().unwrap();
        let found = held.iter().find(|(record, _)| &record.name == name);
        Ok(found.map(|(record, _)| record.kind))
    }

    fn revoke(&self, name: &Slug) -> Result<(), TokenError> {
        self.failing()?;
        let mut held = self.held.lock().unwrap();
        let before = held.len();
        held.retain(|(record, _)| &record.name != name);
        match held.len() < before {
            true => Ok(()),
            false => Err(TokenError::NoSuchToken(name.clone())),
        }
    }

    fn list(&self) -> Result<Vec<TokenRecord>, TokenError> {
        self.failing()?;
        let held = self.held.lock().unwrap();
        Ok(held.iter().map(|(record, _)| record.clone()).collect())
    }

    fn verify(&self, presented: &str) -> Result<Option<Slug>, TokenError> {
        self.failing()?;
        let held = self.held.lock().unwrap();
        Ok(held
            .iter()
            .find(|(_, secret)| secret == presented)
            .map(|(record, _)| record.name.clone()))
    }
}
