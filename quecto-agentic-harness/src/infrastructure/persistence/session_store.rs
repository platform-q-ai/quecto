use crate::application::sessions::dto::SessionListQuery;
use crate::application::sessions::ports::{SessionLoad, SessionStore};
use crate::domain::conversation::stored_images::MessageImageRefs;
use crate::domain::message::{Message, Role};
use crate::domain::session::{Session, SessionSummary};
use crate::domain::session_identity::SessionIdentity;
use crate::domain::{error::DomainError, workflow::WorkflowRunPersisted};
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use super::session_images::ImageSidecarStore;
use super::session_images::{SessionImages, Written, leave_unloaded};
use super::session_layout::FlatSessionLayout;
use std::sync::Arc;

/// What a write returns: what it wrote, for the image sidecars (#2424).
type WriteResult = Result<Written, DomainError>;

/// The file-backed session store: JSON/JSONL records under the flat layout
/// (`FlatSessionLayout` owns every path; this adapter owns the I/O).
#[derive(Debug)]
pub struct FileSessionStore {
    layout: FlatSessionLayout,
    ownership: super::session_ownership::SessionOwnershipRegistry,
    summaries: std::sync::Arc<std::sync::Mutex<session_store_list::SummaryCache>>,
    intact: session_store_write::IntactFiles,
    images: SessionImages,
}

#[path = "session_store_catalogue.rs"]
pub(super) mod session_store_catalogue;
#[path = "session_store_home.rs"]
pub(super) mod session_store_home;
#[path = "session_store_list.rs"]
pub(in crate::infrastructure::persistence) mod session_store_list;
#[path = "session_store_ordinals.rs"]
mod session_store_ordinals;
#[path = "session_store_records.rs"]
pub(super) mod session_store_records;
#[path = "session_store_write.rs"]
mod session_store_write;
use session_store_ordinals::{assign_missing_ordinals, messages_with_assigned_ordinals};
use session_store_records::*;
use session_store_write::{append_known_delta, append_or_compact, compact_or_append_delta};
#[cfg(test)]
use session_store_write::{append_record, persisted_prefix_changed, write_compacted};

impl FileSessionStore {
    /// A store over `layout`'s flat directory.
    pub fn new(layout: FlatSessionLayout) -> Self {
        Self {
            images: SessionImages::default(),
            layout,
            summaries: Default::default(),
            ownership: super::session_ownership::SessionOwnershipRegistry::default(),
            intact: Default::default(),
        }
    }

    /// Keep the images of saved sessions with `sidecars` (#2424); without
    /// them, images are named by reference only, and a reload sends markers.
    pub fn with_image_sidecars(mut self, sidecars: Arc<dyn ImageSidecarStore>) -> Self {
        self.images = SessionImages::over(sidecars);
        self
    }

    #[cfg(feature = "test-support")]
    pub fn summary_transcript_reads(&self) -> usize {
        self.summaries.lock().expect("summary cache lock").reads
    }

    fn claim_key(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        self.ownership.claim(&self.layout, identity)
    }

    fn session_path(&self, identity: &SessionIdentity) -> PathBuf {
        self.layout.session_file(identity)
    }

    pub async fn save_clean_delta(
        &self,
        identity: &SessionIdentity,
        messages: &[Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Result<(), DomainError> {
        self.claim_key(identity)?;
        if messages.is_empty() && workflow_run.is_none() {
            return self.delete_session_file_if_present(identity).await;
        }
        self.ensure_dir().await?;
        self.images.store(identity, messages).await;
        let path = self.session_path(identity);
        let target = &path;
        let written = self
            .tracked(&path, |appendable| async move {
                let must_compact = previously_persisted == 0
                    || !appendable
                    || previously_persisted > messages.len()
                    || !target.exists()
                    || !is_jsonl_session_file(target).await?;
                compact_or_append_delta(
                    target,
                    identity,
                    messages,
                    previously_persisted,
                    workflow_run.as_ref(),
                    must_compact,
                )
                .await
            })
            .await?;
        self.images.collect(identity, written).await;
        Ok(())
    }

    /// The transcript stored for `identity`, as read; `None` when there is none.
    async fn read_parsed(
        &self,
        identity: &SessionIdentity,
    ) -> Result<Option<ParsedSession>, DomainError> {
        let path = self.session_path(identity);
        if !path.exists() {
            return Ok(None);
        }
        let (data, before) = self.intact.read(&path).await?;
        let parsed = parse_session_records(&data)
            .map_err(|e| DomainError::Session(format!("failed to parse session: {}", e)))?;
        self.intact
            .observe_read(&path, before, data.len(), parsed.intact)
            .await;
        Ok(Some(parsed))
    }

    async fn ensure_dir(&self) -> Result<(), DomainError> {
        tokio::fs::create_dir_all(self.layout.sessions_dir())
            .await
            .map_err(|e| DomainError::Session(format!("failed to create sessions dir: {}", e)))
    }
}

impl SessionStore for FileSessionStore {
    fn claim(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        self.claim_key(identity)
    }

    fn release(&self, identity: &SessionIdentity) {
        self.intact.forget(&self.session_path(identity));
        self.ownership.release(identity);
    }

    fn load(&self, identity: &SessionIdentity) -> SessionLoad<'_> {
        let identity = identity.clone();
        Box::pin(async move {
            let Some(parsed) = self.read_parsed(&identity).await? else {
                return Ok(None);
            };
            let mut session = parsed.session;
            self.images
                .restore(&identity, &mut session.messages, parsed.images)
                .await;
            Ok(Some(session))
        })
    }

    fn load_transcript<'a>(&'a self, identity: &'a SessionIdentity) -> SessionLoad<'a> {
        Box::pin(async move {
            let Some(parsed) = self.read_parsed(identity).await? else {
                return Ok(None);
            };
            let mut session = parsed.session;
            leave_unloaded(&mut session.messages, parsed.images);
            Ok(Some(session))
        })
    }

    fn save<'a>(
        &'a self,
        session: &'a Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + 'a>> {
        let path = self.session_path(&session.key);
        Box::pin(async move {
            self.claim_key(&session.key)?;
            if session.messages.is_empty()
                && session.workflow_run.is_none()
                && session.subagent_roster.is_empty()
            {
                return self.delete_session_file_if_present(&session.key).await;
            }
            self.ensure_dir().await?;
            self.images.store(&session.key, &session.messages).await;
            let written = self
                .tracked(&path, |appendable| {
                    append_or_compact(&path, session, appendable)
                })
                .await?;
            self.images.collect(&session.key, written).await;
            Ok(())
        })
    }

    fn save_delta<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let path = self.session_path(identity);
        Box::pin(async move {
            self.claim_key(identity)?;
            if messages.is_empty() && workflow_run.is_none() {
                return self.delete_session_file_if_present(identity).await;
            }
            self.ensure_dir().await?;
            self.images.store(identity, messages).await;
            let written = self
                .tracked(&path, |appendable| {
                    append_known_delta(
                        &path,
                        identity,
                        messages,
                        previously_persisted,
                        workflow_run.as_ref(),
                        appendable,
                    )
                })
                .await?;
            self.images.collect(identity, written).await;
            Ok(())
        })
    }

    fn save_clean_delta<'a>(
        &'a self,
        identity: &'a SessionIdentity,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<WorkflowRunPersisted>,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async move {
            self.claim_key(identity)?;
            if messages.is_empty() && workflow_run.is_none() {
                return self.delete_session_file_if_present(identity).await;
            }
            FileSessionStore::save_clean_delta(
                self,
                identity,
                messages,
                previously_persisted,
                workflow_run,
            )
            .await
        })
    }

    fn exists(
        &self,
        identity: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + '_>> {
        let path = self.session_path(identity);
        Box::pin(async move { Ok(path.exists()) })
    }

    fn list(
        &self,
        query: &SessionListQuery,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionSummary>, DomainError>> + Send + '_>> {
        let query = query.clone();
        Box::pin(async move { Ok(self.walk(&query).await?.0) })
    }
}

/// The session `data` holds, its images named but not read (#2424).
fn parse_session_data(data: &str) -> Result<Session, serde_json::Error> {
    let parsed = parse_session_records(data)?;
    let mut session = parsed.session;
    leave_unloaded(&mut session.messages, parsed.images);
    Ok(session)
}

/// A transcript as read (#2424): the session, its messages' image
/// references by message index (restored from the sidecars on load), and
/// whether every record was read and the last one is terminated: an append
/// may follow only an intact file.
struct ParsedSession {
    session: Session,
    images: BTreeMap<usize, MessageImageRefs>,
    intact: bool,
}

impl ParsedSession {
    /// The record the message at `index` was read from.
    fn record_at(&self, index: usize) -> MessageRecord {
        static NO_IMAGES: MessageImageRefs = MessageImageRefs::NONE;
        let images = self.images.get(&index).unwrap_or(&NO_IMAGES);
        record_with_images(&self.session.messages[index], images)
    }
}

/// The session `data` holds; see [`ParsedSession`].
fn parse_session_records(data: &str) -> Result<ParsedSession, serde_json::Error> {
    if let Ok(file) = serde_json::from_str::<SessionFile>(data) {
        // One snapshot line (a legacy plain-JSON file is compacted anyway).
        let (session, images) = session_from_file_and_images(file);
        let intact = data.ends_with('\n');
        return Ok(ParsedSession {
            session,
            images,
            intact,
        });
    }
    let mut intact = data.ends_with('\n');

    let mut session: Option<Session> = None;
    let mut images = BTreeMap::new();
    let mut parsed_any = false;
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let record: SessionRecord = match serde_json::from_str(line) {
            Ok(record) => record,
            Err(err) if parsed_any => {
                tracing::warn!(error = %err, "ignoring incomplete trailing session record");
                intact = false;
                break;
            }
            Err(err) => return Err(err),
        };
        parsed_any = true;
        match record {
            SessionRecord::Snapshot(file) => {
                let (read, read_images) = session_from_file_and_images(file);
                (session, images) = (Some(read), read_images);
            }
            SessionRecord::Append {
                start_index,
                messages,
                workflow_run,
                workflow_run_cleared,
                subagent_roster,
            } => {
                if let Some(session) = &mut session {
                    if let Some(start_index) = start_index {
                        if start_index != session.messages.len() {
                            tracing::warn!(
                                start_index,
                                current_len = session.messages.len(),
                                "ignoring out-of-order session append record"
                            );
                            intact = false;
                            break;
                        }
                    }
                    for record in messages {
                        let (message, refs) = record_to_message_and_images(record);
                        if !refs.is_empty() {
                            images.insert(session.messages.len(), refs);
                        }
                        session.messages.push(message);
                    }
                    if workflow_run_cleared {
                        session.workflow_run = None;
                    } else if workflow_run.is_some() {
                        session.workflow_run = workflow_run;
                    }
                    if let Some(roster) = subagent_roster {
                        session.subagent_roster = roster;
                    }
                }
            }
        }
    }
    let session = session
        .map(session_store_ordinals::with_assigned_ordinals)
        .unwrap_or_else(|| Session::new(SessionIdentity::ephemeral()));
    Ok(ParsedSession {
        session,
        images,
        intact,
    })
}

/// A snapshot's session, its images named but not read (the bounded read).
fn session_from_file(file: SessionFile) -> Session {
    let (mut session, images) = session_from_file_and_images(file);
    leave_unloaded(&mut session.messages, images);
    session
}

/// A snapshot's session, and its messages' image references by index.
fn session_from_file_and_images(file: SessionFile) -> (Session, BTreeMap<usize, MessageImageRefs>) {
    let mut images = BTreeMap::new();
    let mut messages = Vec::with_capacity(file.messages.len());
    for (index, record) in file.messages.into_iter().enumerate() {
        let (message, refs) = record_to_message_and_images(record);
        if !refs.is_empty() {
            images.insert(index, refs);
        }
        messages.push(message);
    }
    let session = Session {
        key: SessionIdentity::from_persisted_key(file.key),
        messages: assign_missing_ordinals(messages),
        workflow_run: file.workflow_run,
        subagent_roster: file.subagent_roster,
    };
    (session, images)
}

async fn is_jsonl_session_file(path: &Path) -> Result<bool, DomainError> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| DomainError::Session(format!("failed to read session: {e}")))?;
    let mut prefix = [0_u8; 64];
    let len = file
        .read(&mut prefix)
        .await
        .map_err(|e| DomainError::Session(format!("failed to read session: {e}")))?;
    let prefix = std::str::from_utf8(&prefix[..len]).unwrap_or("");
    Ok(prefix.trim_start().starts_with(r#"{"type":"#))
}

fn role_to_str(role: &Role) -> &str {
    role.as_str()
}

fn str_to_role(s: &str) -> Role {
    match s {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

/// Extract the raw title datum: the session's first user message, trimmed.
/// Returns an empty string when there is none, bounded to a transport-safe
/// length (no ellipsis). Display truncation and the "(untitled)" placeholder
/// are applied by the interface/display layer, not by persistence.
fn first_user_message(messages: &[MessageHeader<'_>]) -> String {
    const TRANSPORT_CHAR_CAP: usize = 200;
    messages
        .iter()
        .find(|m| matches!(str_to_role(&m.role), Role::User))
        .map(|m| m.content.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(TRANSPORT_CHAR_CAP).collect())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "session_store_chat_tests.rs"]
mod chat_tests;
#[cfg(test)]
#[path = "session_store_cov_tests.rs"]
mod cov_tests;
#[cfg(test)]
#[path = "session_store_metadata_tests.rs"]
mod metadata_tests;
#[cfg(test)]
#[path = "session_store_subagent_roster_tests.rs"]
mod subagent_roster_tests;
#[cfg(test)]
#[path = "session_store_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "session_store_torn_tests.rs"]
mod torn_tests;
#[cfg(test)]
#[path = "session_store_workflow_tests.rs"]
mod workflow_tests;
