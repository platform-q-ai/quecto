//! The external-agent capability's use cases (#2287): one struct per file,
//! constructed only in composition.

mod drive_external_agent_session;

pub use self::drive_external_agent_session::DriveExternalAgentSession;
