//! The `scope` field of the session listing and search commands (#2370),
//! mapped onto the sessions capability's DTO.
use crate::interface::uds::wire::protocol::SessionListScopeCommand;

impl From<SessionListScopeCommand> for crate::application::sessions::dto::SessionListScope {
    fn from(scope: SessionListScopeCommand) -> Self {
        match scope {
            SessionListScopeCommand::Local => Self::Local,
            SessionListScopeCommand::Global => Self::Global,
        }
    }
}
