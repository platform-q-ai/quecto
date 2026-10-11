//! Clearing a file the board set aside (#2477): one commit without it,
//! every other file carried forward as a write carries it, made as the
//! store's committer so the board records who cleared it.
//!
//! A `.json` name listed more than once (a hand edit git rejects) is
//! cleared only by naming the object to keep (`keep`): that one stays as
//! the name's file and the rest go, so nothing is dropped unseen. Names
//! are cleared one at a time: another name still listed more than once is
//! carried forward exactly as it was, every entry of it, so clearing one
//! never decides another.
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::TREE;

/// A tree entry as git lists it: mode, type, object and raw name.
struct RawEntry {
    mode: String,
    kind: String,
    oid: String,
    name: Vec<u8>,
}

impl GitBoardStore {
    pub(super) fn clear(
        &self,
        expected: &BoardHead,
        path: &str,
        keep: Option<&str>,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        line("message", message, 200)
            .map_err(|error| BoardStoreError::InvalidWrite(error.to_string()))?;
        let base = self.base(expected)?;
        let board = self.board_at(&base)?;
        if !board.quarantined.iter().any(|file| file.path == path) {
            // From a head the board moved past, that is a lost race, as a
            // write's is: the file may be set aside now.
            return match self.remote_head(BOARD_REF)?.as_deref() == Some(base.as_str()) {
                true => Ok(BoardWrite::Refused(format!("{path} is not set aside"))),
                false => Ok(BoardWrite::Lost),
            };
        }
        let name = path.strip_prefix("tasks/").unwrap_or(path).to_string();
        let mut cleared = (*board).clone();
        cleared.quarantined.retain(|file| file.path != path);
        let repeated = board
            .repeated
            .iter()
            .find(|(repeated, _)| *repeated == name)
            .map(|(_, oids)| oids.clone());
        cleared.repeated.retain(|(repeated, _)| *repeated != name);
        match (repeated, keep) {
            (Some(oids), Some(keep)) if oids.iter().any(|oid| oid == keep) => {
                if !self.is_blob(keep)? {
                    return Ok(BoardWrite::Refused(format!("{keep} is not a file")));
                }
                cleared
                    .kept
                    .insert(name.clone(), format!("{REGULAR_FILE} blob {keep}"));
            }
            (Some(oids), Some(keep)) => {
                return Ok(BoardWrite::Refused(format!(
                    "{keep} is not one of {path}'s objects ({})",
                    oids.join(", ")
                )));
            }
            (Some(oids), None) => {
                return Ok(BoardWrite::Refused(format!(
                    "{path} is named more than once: name the object to keep, one of {}",
                    oids.join(", ")
                )));
            }
            (None, Some(_)) => {
                return Ok(BoardWrite::Refused(format!(
                    "{path} is named once; an object to keep is for a name named more than once"
                )));
            }
            (None, None) => {
                cleared.kept.remove(&name);
            }
        }
        let built = self.tasks_tree(&cleared, &[])?;
        let tasks = self.with_repeated(&base, &built, &cleared.repeated)?;
        let readme = self.blob(README)?;
        let root = BTreeMap::from([
            (
                "README.md".to_string(),
                format!("{REGULAR_FILE} blob {readme}"),
            ),
            ("tasks".to_string(), format!("{TREE} tree {tasks}")),
        ]);
        let tree = self.tree(&root)?;
        let args = [
            "commit-tree",
            tree.as_str(),
            "-p",
            base.as_str(),
            "-m",
            message,
        ];
        let commit = self.git.text(&args, None).map_err(unavailable)?;
        match self.push(Some(&base), &commit)? {
            Pushed::Landed => Ok(BoardWrite::Written(BoardHead(commit))),
            Pushed::Lost => Ok(BoardWrite::Lost),
            Pushed::Refused(reason) => Ok(BoardWrite::Refused(reason)),
        }
    }

    /// `built` (a tasks tree a write made) with every entry of each name
    /// still `repeated` restored exactly as `base` lists them.
    fn with_repeated(
        &self,
        base: &str,
        built: &str,
        repeated: &[(String, Vec<String>)],
    ) -> Result<String, BoardStoreError> {
        if repeated.is_empty() {
            return Ok(built.to_string());
        }
        let names: BTreeSet<&[u8]> = repeated.iter().map(|(name, _)| name.as_bytes()).collect();
        let original = self.raw_entries(&format!("{base}:tasks"))?;
        let mut entries: Vec<RawEntry> = self
            .raw_entries(built)?
            .into_iter()
            .filter(|entry| !names.contains(entry.name.as_slice()))
            .collect();
        entries.extend(
            original
                .into_iter()
                .filter(|entry| names.contains(entry.name.as_slice())),
        );
        let mut input = Vec::new();
        for entry in &entries {
            input.extend(format!("{} {} {}\t", entry.mode, entry.kind, entry.oid).as_bytes());
            input.extend(&entry.name);
            input.push(0);
        }
        if input.len() > self.limits.tree_bytes {
            return Err(BoardStoreError::InvalidWrite(format!(
                "the board would hold a tree of {} bytes; a read takes at most {}",
                input.len(),
                self.limits.tree_bytes
            )));
        }
        object_id(
            &self
                .git
                .text(&["mktree", "-z"], Some(&input))
                .map_err(unavailable)?,
        )
    }

    /// The entries of `tree` as git lists them, names raw.
    fn raw_entries(&self, tree: &str) -> Result<Vec<RawEntry>, BoardStoreError> {
        let listing = self
            .git
            .ok_up_to(&["ls-tree", "-z", tree], None, self.limits.tree_bytes * 2)
            .map_err(unavailable)?;
        listing
            .split(|byte| *byte == 0)
            .filter(|row| !row.is_empty())
            .map(|row| {
                let malformed =
                    || BoardStoreError::Unavailable(format!("ls-tree listed {:?}", quoted(row)));
                let tab = row
                    .iter()
                    .position(|byte| *byte == b'\t')
                    .ok_or_else(malformed)?;
                let meta = String::from_utf8_lossy(&row[..tab]).into_owned();
                match meta.split(' ').collect::<Vec<_>>().as_slice() {
                    [mode, kind, oid] => Ok(RawEntry {
                        mode: (*mode).into(),
                        kind: (*kind).into(),
                        oid: object_id(oid)?,
                        name: row[tab + 1..].to_vec(),
                    }),
                    _ => Err(malformed()),
                }
            })
            .collect()
    }

    fn is_blob(&self, oid: &str) -> Result<bool, BoardStoreError> {
        let oid = object_id(oid)?;
        let args = ["cat-file", "-t", oid.as_str()];
        let output = self
            .git
            .local(&args, None, SHORT_OUTPUT)
            .map_err(unavailable)?;
        Ok(output.status.success() && output.stdout == b"blob\n")
    }
}
