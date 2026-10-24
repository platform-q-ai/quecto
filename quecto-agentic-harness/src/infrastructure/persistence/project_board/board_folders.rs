//! The board's folders besides `tasks/` (#2481 S9): `plans`, `runs`,
//! `changes`, `reviews` and `discussions`, each of one kind's files, and
//! the `archive` tree, room left for later (M5): type-checked as a tree,
//! never read, outside every bound.
//!
//! A folder's file is read when its name spells a key, `<task>.<n>.md` (a
//! number from 1 to 999 with no leading zero) or a discussion's `<id>.md`,
//! and it decodes as its kind's canonical file of that very key. Every
//! other entry is judged as one in `tasks/` is (`board_entries`): set
//! aside under the key its name spells, if any, and carried or dropped by
//! the same rules.
//!
//! A write (S10a) carries a folder it leaves as it was read as the very
//! tree, and rebuilds one it writes in, or one holding an entry a write
//! drops, from what it carries (`board_writes`): each file read and each
//! file set aside but kept, by its object, so a reason here says what the
//! next write does with a file, as one in `tasks/` does.
use super::entries::{Disposition, Naming, TaskFile, disposition};
use super::{
    Board, Entry, LeftOnRemote, Limits, MAX_TASK_FILE_BYTES, REGULAR_FILE, TreeCopy, Unread,
    capped, contents, entry_path, kind_of_mode, kinds_in, listed, omitted_in, sanitised, too_large,
};
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardHead, BoardSnapshot, QuarantinedFile,
};
use crate::domain::project_board::entities::change::MAX_CHANGE_NUMBER;
use crate::domain::project_board::entities::plan::MAX_PLAN_VERSION;
use crate::domain::project_board::entities::run::MAX_RUN_NUMBER;
use crate::domain::project_board::value_objects::slug::{MAX_SLUG_BYTES, Slug};
use crate::infrastructure::persistence::project_board::artifacts::change::{
    CHANGE_LIMITS, ChangeCodec,
};
use crate::infrastructure::persistence::project_board::artifacts::discussion::{
    DISCUSSION_LIMITS, DiscussionCodec,
};
use crate::infrastructure::persistence::project_board::artifacts::plan::{PLAN_LIMITS, PlanCodec};
use crate::infrastructure::persistence::project_board::artifacts::review::{
    REVIEW_LIMITS, ReviewCodec,
};
use crate::infrastructure::persistence::project_board::artifacts::run::{RUN_LIMITS, RunCodec};
use crate::infrastructure::persistence::project_board::artifacts::{CodecError, decode};
use std::collections::BTreeMap;

const MIB: usize = 1024 * 1024;

/// The archive's name at a board's root.
pub const ARCHIVE: &str = "archive";

/// The size from which a partial clone's fetch of a folder leaves a blob
/// on the remote: one past the largest file a read reads (a plan's), so
/// every file a read may read is fetched and none past every cap is.
pub const FILTER_BYTES: usize = PLAN_LIMITS.file_bytes + 1;
const _: () = assert!(
    MAX_TASK_FILE_BYTES < FILTER_BYTES
        && RUN_LIMITS.file_bytes < FILTER_BYTES
        && CHANGE_LIMITS.file_bytes < FILTER_BYTES
        && REVIEW_LIMITS.file_bytes < FILTER_BYTES
        && DISCUSSION_LIMITS.file_bytes < FILTER_BYTES,
    "every file a read reads is fetched"
);

/// The most entries one read takes in all the board's folders.
pub const MAX_BOARD_ENTRIES: usize = 16_384;

/// One folder of a kind's files besides `tasks/`.
pub(super) struct Folder {
    pub naming: Naming,
    /// The key a file's name spells, from the name without its `.md`.
    pub spell: fn(&str) -> Option<ArtifactKey>,
    /// The artifact a file's bytes hold, by the kind's codec.
    pub decode: fn(&[u8]) -> Result<BoardArtifact, CodecError>,
    /// The most entries a read takes in it.
    pub max_entries: usize,
    /// The most file bytes a read takes in it.
    pub max_bytes: usize,
}

/// The longest name a numbered folder keeps: the longest id, the most
/// digits and `.md`.
const MAX_NUMBERED_NAME: usize = MAX_SLUG_BYTES + ".999.md".len();

/// How a numbered folder names its files: `<task>.<n>.md`.
const fn numbered_naming(
    folder: &'static str,
    noun: &'static str,
    spelled: &'static str,
    file_bytes: usize,
) -> Naming {
    Naming {
        folder,
        noun,
        spelled,
        pattern: "<task>.<n>.md",
        max_name: MAX_NUMBERED_NAME,
        file_bytes,
    }
}

/// The folders, in the order the plan lists them.
pub(super) static FOLDERS: [Folder; 5] = [
    Folder {
        naming: numbered_naming("plans", "plan", "plan name", PLAN_LIMITS.file_bytes),
        spell: |stem| numbered(stem, MAX_PLAN_VERSION).map(|(id, n)| ArtifactKey::Plan(id, n)),
        decode: |bytes| decode::<PlanCodec>(bytes).map(BoardArtifact::Plan),
        max_entries: 4_096,
        max_bytes: 24 * MIB,
    },
    Folder {
        naming: numbered_naming("runs", "run", "run name", RUN_LIMITS.file_bytes),
        spell: |stem| numbered(stem, MAX_RUN_NUMBER).map(|(id, n)| ArtifactKey::Run(id, n)),
        decode: |bytes| decode::<RunCodec>(bytes).map(BoardArtifact::Run),
        max_entries: 4_096,
        max_bytes: 16 * MIB,
    },
    Folder {
        naming: numbered_naming("changes", "change", "change name", CHANGE_LIMITS.file_bytes),
        spell: |stem| numbered(stem, MAX_CHANGE_NUMBER).map(|(id, n)| ArtifactKey::Change(id, n)),
        decode: |bytes| decode::<ChangeCodec>(bytes).map(BoardArtifact::Change),
        max_entries: 4_096,
        max_bytes: 4 * MIB,
    },
    Folder {
        naming: Naming {
            pattern: "<task>.<change>.md",
            ..numbered_naming("reviews", "review", "review name", REVIEW_LIMITS.file_bytes)
        },
        spell: |stem| numbered(stem, MAX_CHANGE_NUMBER).map(|(id, n)| ArtifactKey::Review(id, n)),
        decode: |bytes| decode::<ReviewCodec>(bytes).map(BoardArtifact::Review),
        max_entries: 4_096,
        max_bytes: 16 * MIB,
    },
    Folder {
        naming: Naming {
            folder: "discussions",
            noun: "discussion",
            spelled: "discussion name",
            pattern: "<id>.md",
            max_name: MAX_SLUG_BYTES + ".md".len(),
            file_bytes: DISCUSSION_LIMITS.file_bytes,
        },
        spell: |stem| Slug::parse(stem).ok().map(ArtifactKey::Discussion),
        decode: |bytes| decode::<DiscussionCodec>(bytes).map(BoardArtifact::Discussion),
        max_entries: 2_048,
        max_bytes: 16 * MIB,
    },
];

/// The trees a board's root holds by name: `tasks`, each folder and the
/// archive. Any other tree is no board's.
pub(super) fn root_tree(name: &str) -> bool {
    name == "tasks" || name == ARCHIVE || FOLDERS.iter().any(|folder| folder.naming.folder == name)
}

/// Whether `name` is one of the folders besides `tasks/`.
pub fn is_folder(name: &str) -> bool {
    FOLDERS.iter().any(|folder| folder.naming.folder == name)
}

/// The bytes a read reads of `copy` kept as `name` in the folder named
/// `folder`: a regular file within its kind's cap whose name spells a key.
pub fn read_size_in(folder: &str, name: &str, copy: &TreeCopy) -> Option<usize> {
    let found = FOLDERS.iter().find(|found| found.naming.folder == folder)?;
    let spelled = name.strip_suffix(".md").and_then(found.spell);
    spelled.and(copy.read_size(found.naming.file_bytes))
}

/// The task and number `stem` spells: `<id>.<n>`, `n` from 1 to `max`
/// written with no leading zero.
fn numbered(stem: &str, max: u32) -> Option<(Slug, u32)> {
    let (id, number) = stem.rsplit_once('.')?;
    let digits = number.as_bytes();
    let shaped = digits.len() <= 3
        && digits.first().is_some_and(|b| (b'1'..=b'9').contains(b))
        && digits.iter().all(u8::is_ascii_digit);
    let number: u32 = shaped.then(|| number.parse().ok()).flatten()?;
    match number <= max {
        true => Some((Slug::parse(id).ok()?, number)),
        false => None,
    }
}

/// A folder as a read found it: carried by a write that leaves it as it
/// was as this tree, and rebuilt from `files` by one that does not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CarriedTree {
    pub oid: String,
    /// Its entries, all of them.
    pub entries: usize,
    /// The bytes of its files a read reads.
    pub bytes: usize,
    /// What a write carries, by name (`<mode> <type> <oid>`): each file
    /// read, and each set aside but kept. Every other entry is dropped.
    pub files: BTreeMap<String, String>,
    /// The bytes of each file a read reads, by name: some of `files`.
    pub sizes: BTreeMap<String, usize>,
    /// Names listed more than once among the copies a write carries, each
    /// with every copy: a tree git rejects (`Board::repeated`), so a write
    /// into the folder waits until a clear resolves them, and any other
    /// write carries the folder as its very tree, making no tree of them.
    pub repeated: Vec<(String, Vec<TreeCopy>)>,
}

impl CarriedTree {
    /// Whether a write carries every entry of it, so one that leaves it as
    /// it was carries its very tree; otherwise it rebuilds it.
    pub fn carries_every_entry(&self) -> bool {
        self.files.len() == self.entries
    }
}

/// What a read of one folder found.
pub(super) struct FolderRead {
    pub tree: CarriedTree,
    /// Blobs a partial clone's filter left on the remote.
    pub omitted: Vec<String>,
    pub artifacts: Vec<BoardArtifact>,
    pub quarantined: Vec<QuarantinedFile>,
}

/// One folder scanned: listed, every entry judged by its type and size
/// alone, and the files still to read (key, entry, size).
pub(super) struct FolderScan {
    folder: &'static Folder,
    tree: CarriedTree,
    quarantined: Vec<QuarantinedFile>,
    to_read: Vec<(ArtifactKey, Entry, usize)>,
    omitted: Vec<String>,
}

impl FolderScan {
    /// The file bytes a load reads.
    pub(super) fn bytes(&self) -> usize {
        self.tree.bytes
    }

    /// The entries the scan listed.
    pub(super) fn entries(&self) -> usize {
        self.tree.entries
    }
}

/// `entry` of `folder` set aside: under `key`, if any, and `carried` when
/// a write would carry it.
fn aside(
    folder: &Folder,
    entry: &Entry,
    key: Option<ArtifactKey>,
    reason: &str,
    carried: bool,
) -> QuarantinedFile {
    let path = entry_path(folder.naming.folder, &entry.raw, &entry.oid);
    if carried {
        // A carried name shows as it is, as one in `tasks/` does.
        let name = format!("{}/{}", folder.naming.folder, entry.name);
        assert_eq!(path, name, "a carried name shows as it is");
    }
    QuarantinedFile {
        path,
        key,
        reason: capped(&sanitised(reason.as_bytes())),
        object: entry.oid.clone(),
        carried,
    }
}

/// The folder `folder` of tree `tree`, scanned: listed within its entries,
/// every entry judged, and the bytes of its files to read within its own
/// bound. No file's bytes are read.
///
/// # Errors
/// Objects this clone does not hold; git fails, or the folder is past its
/// bounds.
pub(super) fn scan_folder(
    git: &impl super::Reader,
    tree: &str,
    folder: &'static Folder,
    limits: Limits,
    left: &LeftOnRemote,
) -> Result<FolderScan, Unread> {
    let name = folder.naming.folder;
    let entries = listed(git, tree, limits.tree_bytes)?;
    if entries.len() > folder.max_entries {
        return Err(too_large(format!(
            "the board's {name} tree holds {} entries; the most is {}",
            entries.len(),
            folder.max_entries
        )));
    }
    let kinds = kinds_in(git, tree, &entries, left)?;
    let mut counts: BTreeMap<&[u8], usize> = BTreeMap::new();
    for entry in entries.iter().filter(|entry| entry.carried(&kinds)) {
        *counts.entry(entry.raw.as_slice()).or_default() += 1;
    }
    let mut quarantined = vec![];
    let mut to_read = vec![];
    let (mut files, mut sizes) = (BTreeMap::new(), BTreeMap::new());
    for entry in &entries {
        let found = match kinds.get(&entry.oid) {
            Some(super::Found::Present(object)) => Some(object),
            Some(super::Found::Missing) | None => None,
        };
        let file = entry
            .name
            .strip_suffix(".md")
            .map(|stem| ((folder.spell)(stem), TaskFile::Markdown));
        let count = counts.get(entry.raw.as_slice()).copied().unwrap_or(0);
        match disposition(entry, found, count, file, &folder.naming) {
            Disposition::Read(key) => {
                let size = found.map_or(0, |o| o.size);
                let file = format!("{REGULAR_FILE} blob {}", entry.oid);
                assert!(
                    files.insert(entry.name.clone(), file).is_none(),
                    "read once"
                );
                sizes.insert(entry.name.clone(), size);
                to_read.push((key, entry.clone(), size));
            }
            Disposition::Aside { id, reason, kept } => {
                if kept {
                    let kind = kind_of_mode(&entry.mode);
                    assert!(
                        matches!(kind, Some("blob" | "tree")),
                        "a kept entry's mode names a blob or a tree"
                    );
                    let kind = kind.unwrap_or_default();
                    let file = format!("{} {kind} {}", entry.mode, entry.oid);
                    assert!(
                        files.insert(entry.name.clone(), file).is_none(),
                        "kept once"
                    );
                }
                quarantined.push(aside(folder, entry, id, &reason, kept));
            }
        }
    }
    // Only names a write would carry, as in `tasks/`: a stray is dropped,
    // however often it is named.
    let mut repeated: BTreeMap<String, Vec<TreeCopy>> = BTreeMap::new();
    for entry in &entries {
        let twice = counts.get(entry.raw.as_slice()).copied().unwrap_or(0) > 1;
        let named = entry.kept_name(folder.naming.max_name) && entry.name.ends_with(".md");
        // A twin a write drops (a gitlink, a mode over another type) is
        // no copy.
        let copy = named && twice && entry.carried(&kinds);
        if let (true, Some(super::Found::Present(object))) = (copy, kinds.get(&entry.oid)) {
            repeated
                .entry(entry.name.clone())
                .or_default()
                .push(TreeCopy {
                    mode: entry.mode.clone(),
                    kind: object.kind.clone(),
                    oid: entry.oid.clone(),
                    size: object.size,
                });
        }
    }
    let bytes: usize = to_read.iter().map(|(_, _, size)| size).sum();
    if bytes > folder.max_bytes {
        return Err(too_large(format!(
            "the board's {name} files hold {bytes} bytes; the most read is {}",
            folder.max_bytes
        )));
    }
    let tree = CarriedTree {
        oid: tree.to_string(),
        entries: entries.len(),
        bytes,
        files,
        sizes,
        repeated: repeated.into_iter().collect(),
    };
    Ok(FolderScan {
        folder,
        tree,
        quarantined,
        to_read,
        omitted: omitted_in(&kinds).collect(),
    })
}

/// The folder `scan` found, its files read and decoded.
///
/// # Errors
/// git fails to give the files' bytes.
pub(super) fn load_folder(
    git: &impl super::Reader,
    scan: FolderScan,
) -> Result<FolderRead, Unread> {
    let FolderScan {
        folder,
        tree,
        mut quarantined,
        to_read,
        omitted,
    } = scan;
    let wanted: Vec<(&str, usize)> = to_read
        .iter()
        .map(|(_, entry, size)| (entry.oid.as_str(), *size))
        .collect();
    #[cfg(feature = "test-support")]
    git.witness_artifacts(
        &to_read
            .iter()
            .map(|(_, entry, _)| {
                (
                    format!("{}/{}", folder.naming.folder, entry.name),
                    entry.oid.clone(),
                )
            })
            .collect::<Vec<_>>(),
    );
    let contents = contents(git, &wanted)?;
    let mut artifacts = vec![];
    for (key, entry, _) in to_read {
        // Two files may share one blob, so each is looked up, not taken.
        let bytes = contents.get(&entry.oid).cloned().unwrap_or_default();
        let noun = folder.naming.noun;
        let read = (folder.decode)(&bytes)
            .map_err(|error| format!("{noun} file: {error}"))
            .and_then(|artifact| match artifact.key() == key {
                true => Ok(artifact),
                false => Err(format!("holds {}, not {key}", artifact.key())),
            });
        match read {
            Ok(artifact) => artifacts.push(artifact),
            Err(reason) => quarantined.push(aside(folder, &entry, Some(key), &reason, true)),
        }
    }
    quarantined.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(FolderRead {
        tree,
        omitted,
        artifacts,
        quarantined,
    })
}

impl Board {
    /// `self` with the folders `read` and the archive `archive` beside its
    /// tasks.
    pub(super) fn with_folders(
        mut self,
        read: Vec<(&'static str, FolderRead)>,
        archive: Option<String>,
    ) -> Self {
        for (name, folder) in read {
            self.artifacts.extend(folder.artifacts);
            self.omitted.extend(folder.omitted);
            self.quarantined.extend(folder.quarantined);
            self.folders.insert(name, folder.tree);
        }
        self.quarantined
            .sort_by(|left, right| left.path.cmp(&right.path));
        // An artifact is read or set aside, never both, and read once.
        let read: std::collections::BTreeSet<ArtifactKey> =
            self.artifacts.iter().map(BoardArtifact::key).collect();
        assert_eq!(read.len(), self.artifacts.len(), "an artifact is read once");
        assert!(
            self.quarantined
                .iter()
                .filter_map(|file| file.key.as_ref())
                .all(|key| !read.contains(key)),
            "an artifact read is never one set aside"
        );
        self.archive = archive;
        self
    }

    /// The board as a read hands it over, at `head`: every artifact read
    /// in its kind's list, and every file set aside.
    pub fn snapshot(&self, head: BoardHead) -> BoardSnapshot {
        let mut snapshot = BoardSnapshot {
            head,
            tasks: self.tasks.values().map(|held| held.task.clone()).collect(),
            plans: vec![],
            runs: vec![],
            changes: vec![],
            reviews: vec![],
            discussions: vec![],
            quarantined: self.quarantined.clone(),
        };
        for artifact in &self.artifacts {
            match artifact.clone() {
                BoardArtifact::Plan(plan) => snapshot.plans.push(plan),
                BoardArtifact::Run(run) => snapshot.runs.push(run),
                BoardArtifact::Change(change) => snapshot.changes.push(change),
                BoardArtifact::Review(review) => snapshot.reviews.push(review),
                BoardArtifact::Discussion(discussion) => snapshot.discussions.push(discussion),
                // Each folder's codec reads its own kind; tasks are read
                // from `tasks/` alone.
                BoardArtifact::Task(_) => unreachable!("a task is read from tasks/ alone"),
            }
        }
        snapshot
    }
}
