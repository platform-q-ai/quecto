//! Who committed a board commit, read back exactly as recorded.
use super::*;

impl GitBoardStore {
    /// The committer of `commit`, exactly as recorded, fetching the board
    /// first when this clone does not hold it yet.
    ///
    /// # Errors
    /// The commit is not on the board, or git fails.
    pub fn committed_by(&self, commit: &str) -> Result<Identity, BoardStoreError> {
        let commit = object_id(commit)?;
        let args = [
            "log",
            "-1",
            "--no-show-signature",
            "--format=%cn%x00%ce",
            commit.as_str(),
        ];
        let bytes = match self.git.ok(&args, None) {
            Ok(bytes) => bytes,
            Err(GitError::Failed { .. }) => {
                self.fetch()?;
                self.git.ok(&args, None).map_err(unavailable)?
            }
            Err(error) => return Err(unavailable(error)),
        };
        let text = String::from_utf8_lossy(&bytes);
        let (name, email) = text
            .trim_end_matches('\n')
            .split_once('\0')
            .ok_or_else(|| BoardStoreError::Unavailable(format!("no committer in {commit}")))?;
        Ok(Identity {
            name: name.into(),
            email: email.into(),
        })
    }

    /// The remote's fetch URL, as git resolves it (`insteadOf` applied).
    ///
    /// # Errors
    /// No such remote, or git fails.
    pub fn remote_url(&self) -> Result<String, BoardStoreError> {
        self.git
            .text(&["remote", "get-url", "--", self.remote.as_str()], None)
            .map_err(unavailable)
    }
}
