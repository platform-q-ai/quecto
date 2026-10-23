//! SPIKE (board service): a board as the files it holds, path by path,
//! each the very bytes the git branch holds (the existing codecs'
//! canonical markdown). One set of rules turns files into a
//! [`BoardSnapshot`], used by the SQLite store and by the service's
//! client alike, so the wire carries markdown, never a second encoding.
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardHead, BoardSnapshot, QuarantinedFile,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::infrastructure::persistence::project_board::artifacts::{decode, task::TaskCodec};
use crate::infrastructure::persistence::project_board::board_tree::{
    FORMAT, FORMAT_FILE, FORMAT_TREE, decode_folder_file, folder_file,
};
use sha2::{Digest, Sha256};

pub const REGULAR: &str = "100644";
pub const README_PATH: &str = "README.md";

/// The format marker's path.
pub fn format_path() -> String {
    format!("{FORMAT_TREE}/{FORMAT_FILE}")
}

/// The files every write leaves at the root, as the git store regenerates
/// them.
pub fn root_files() -> [(String, Vec<u8>); 2] {
    [
        (
            README_PATH.to_string(),
            crate::infrastructure::persistence::project_board::store::README.to_vec(),
        ),
        (format_path(), FORMAT.to_vec()),
    ]
}

/// One file of the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardFile {
    pub path: String,
    pub mode: String,
    pub bytes: Vec<u8>,
}

/// The board's head: `epoch:revision`. A restore or a promotion starts a
/// new epoch, so a head of an older epoch never matches a newer board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Head {
    pub epoch: u64,
    pub revision: u64,
}

impl Head {
    pub fn text(self) -> String {
        format!("{}:{}", self.epoch, self.revision)
    }

    pub fn board_head(self) -> BoardHead {
        BoardHead(self.text())
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (epoch, revision) = text.split_once(':')?;
        let digits =
            |s: &str| !s.is_empty() && s.len() <= 19 && s.bytes().all(|b| b.is_ascii_digit());
        match (digits(epoch), digits(revision)) {
            (true, true) => Some(Self {
                epoch: epoch.parse().ok()?,
                revision: revision.parse().ok()?,
            }),
            _ => None,
        }
    }
}

/// The object a file's bytes are named by: git's SHA-256 blob id, so a
/// file set aside is named by its content, as on a SHA-256 repository.
pub fn blob_object(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The path of `key`'s file.
///
/// # Errors
/// A key no folder names a file by.
pub fn artifact_path(key: &ArtifactKey, bytes: &[u8]) -> Result<String, String> {
    match key {
        ArtifactKey::Task(id) => Ok(format!("tasks/{}.md", id.as_str())),
        other => {
            folder_file(other, bytes.to_vec()).map(|file| format!("{}/{}", file.folder, file.name))
        }
    }
}

/// What a file at `path` is to a read.
#[derive(Clone)]
pub enum Read {
    /// The root's own files and the archive: carried, never listed.
    Meta,
    Artifact(BoardArtifact),
    Aside(QuarantinedFile),
}

fn aside(
    path: &str,
    bytes: &[u8],
    key: Option<ArtifactKey>,
    reason: String,
    carried: bool,
) -> Read {
    Read::Aside(QuarantinedFile {
        path: path.to_string(),
        key,
        reason,
        object: blob_object(bytes),
        carried,
    })
}

/// What the file at `path` holding `bytes` is.
pub fn read_file(path: &str, mode: &str, bytes: &[u8]) -> Read {
    if path == README_PATH || path == format_path() || path.starts_with("archive/") {
        return Read::Meta;
    }
    let regular = mode == REGULAR;
    let Some((folder, name)) = path.split_once('/') else {
        return aside(
            path,
            bytes,
            None,
            "a stray at the board's root: the next write drops it".into(),
            false,
        );
    };
    let kept_name = !name.contains('/');
    if folder == "tasks" {
        let stem_ext = name.rsplit_once('.');
        let id = stem_ext.and_then(|(stem, _)| Slug::parse(stem).ok());
        return match (stem_ext.map(|(_, ext)| ext), id, kept_name && regular) {
            (Some("md"), Some(id), true) => match decode::<TaskCodec>(bytes) {
                Ok(task) if task.fields().id == id => Read::Artifact(BoardArtifact::Task(task)),
                Ok(task) => {
                    let reason = format!(
                        "task file: holds task {}, not {}",
                        task.fields().id.as_str(),
                        id.as_str()
                    );
                    aside(path, bytes, Some(ArtifactKey::Task(id)), reason, true)
                }
                Err(error) => aside(
                    path,
                    bytes,
                    Some(ArtifactKey::Task(id)),
                    format!("task file: {error}"),
                    true,
                ),
            },
            (Some("json"), Some(id), true) => aside(
                path,
                bytes,
                Some(ArtifactKey::Task(id)),
                "the JSON form older quecto wrote: every write carries it".into(),
                true,
            ),
            (Some("md" | "json"), None, true) => aside(
                path,
                bytes,
                None,
                "a task file whose name spells no task id: every write carries it".into(),
                true,
            ),
            _ => aside(
                path,
                bytes,
                None,
                "not a task file: the next write drops it".into(),
                false,
            ),
        };
    }
    match (kept_name && regular)
        .then(|| decode_folder_file(folder, name, bytes))
        .flatten()
    {
        Some((_, Ok(artifact))) => Read::Artifact(artifact),
        Some((key, Err(reason))) => aside(path, bytes, Some(key), reason, true),
        None => aside(
            path,
            bytes,
            None,
            format!("not a {folder} file: the next write drops it"),
            false,
        ),
    }
}

/// The board `files` hold, at `head`.
pub fn snapshot_of(head: BoardHead, files: &[BoardFile]) -> BoardSnapshot {
    let reads: Vec<Read> = files
        .iter()
        .map(|file| read_file(&file.path, &file.mode, &file.bytes))
        .collect();
    snapshot_from_reads(head, reads)
}

/// The board of files already read (each by [`read_file`]), at `head`.
pub fn snapshot_from_reads(head: BoardHead, reads: Vec<Read>) -> BoardSnapshot {
    let mut snapshot = BoardSnapshot {
        head,
        tasks: vec![],
        plans: vec![],
        runs: vec![],
        changes: vec![],
        reviews: vec![],
        discussions: vec![],
        quarantined: vec![],
    };
    for read in reads {
        match read {
            Read::Meta => {}
            Read::Aside(set_aside) => snapshot.quarantined.push(set_aside),
            Read::Artifact(BoardArtifact::Task(task)) => snapshot.tasks.push(task),
            Read::Artifact(BoardArtifact::Plan(plan)) => snapshot.plans.push(plan),
            Read::Artifact(BoardArtifact::Run(run)) => snapshot.runs.push(run),
            Read::Artifact(BoardArtifact::Change(change)) => snapshot.changes.push(change),
            Read::Artifact(BoardArtifact::Review(review)) => snapshot.reviews.push(review),
            Read::Artifact(BoardArtifact::Discussion(discussion)) => {
                snapshot.discussions.push(discussion);
            }
        }
    }
    snapshot
        .quarantined
        .sort_by(|left, right| left.path.cmp(&right.path));
    snapshot.asserted()
}
