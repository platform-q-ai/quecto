//! Shared inference admission authority adapters (#1679 P3): private
//! directory, singleton lock, durable journal, framed UDS server and client.
pub mod admin_adapter;
pub mod client;
pub mod directory;
pub mod journal;
pub mod link;
pub mod observed_gate;
pub mod process;
pub mod protocol;
pub mod remote_gate;
pub mod secret;
mod server;
mod server_actor;
mod session;

pub use admin_adapter::SocketAuthorityAdmin;
pub use client::{AdminConnection, AuthorityConnection, ClientError, Hello};
pub use directory::{AuthorityDirectory, SingletonLock};
pub use journal::FileJournal;
pub use link::{AuthorityLink, LinkChangeHook, LinkHealth};
pub use observed_gate::{ActivityHook, AdmissionRecorder, ObservedAdmission};
pub use process::{
    AdmissionContext, BindingKind, Negotiation, ProcessAdmission, negotiate,
    read_admission_context, write_admission_context,
};
pub use remote_gate::RemoteAdmission;
pub use secret::RandomSecretSource;
pub use server::{AuthorityServer, ServerError};
