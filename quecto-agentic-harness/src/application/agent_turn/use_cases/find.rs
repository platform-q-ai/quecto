//! Typed invocation of path discovery; delivery and filesystem details stay outside.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Which entries a search lists (#2200); no kind lists both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindEntryKind {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FindRequest {
    pub pattern: String,
    pub path: String,
    pub limit: Option<f64>,
    pub kind: Option<FindEntryKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindPathsRequest {
    pub pattern: String,
    pub path: String,
    pub limit: usize,
    pub kind: Option<FindEntryKind>,
}

/// Entries are complete paths, never raw fragments: relative to the
/// workspace, as grep prints them, or absolute outside it, so each can be
/// passed to read or edit unchanged (#2203). The fd adapter sorts them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindOutput {
    pub entries: Vec<String>,
    /// More matches exist than were returned: the entries are an arbitrary
    /// subset of them, not the first.
    pub result_limit_reached: bool,
    pub incomplete: bool,
    pub diagnostic: Option<String>,
    /// A VCS metadata directory directly under the search root, which the
    /// search skipped, as a path to pass back to search it (#2199 review).
    pub skipped_vcs_dir: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindError {
    Security(String),
    Spawn(String),
    Search(String),
    Io(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindResult {
    pub output: FindOutput,
    pub limit: usize,
    pub kind: Option<FindEntryKind>,
}

pub trait FindPaths: Send + Sync {
    fn find(
        &self,
        request: FindPathsRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FindOutput, FindError>> + Send + '_>>;
}

pub struct FindUseCase {
    paths: Arc<dyn FindPaths>,
}

impl FindUseCase {
    pub fn new(paths: Arc<dyn FindPaths>) -> Self {
        Self { paths }
    }

    pub async fn execute(&self, request: FindRequest) -> Result<FindResult, FindError> {
        // Float-to-integer saturation also defines defensive NaN/infinity handling.
        let limit = request
            .limit
            .map(|value| (value.round() as usize).clamp(1, 100_000))
            .unwrap_or(1000);
        assert!(
            (1..=100_000).contains(&limit),
            "normalized find limit invariant"
        );
        let (pattern, kind) = entry_kind(request.pattern, request.kind)?;
        let output = self
            .paths
            .find(FindPathsRequest {
                pattern,
                path: request.path,
                limit,
                kind,
            })
            .await?;
        Ok(FindResult {
            output,
            limit,
            kind,
        })
    }
}

/// Paths never end in '/' when fd matches them, so a pattern that does is
/// read as what it looks like: directories matching the rest (#2200).
fn entry_kind(
    pattern: String,
    kind: Option<FindEntryKind>,
) -> Result<(String, Option<FindEntryKind>), FindError> {
    match (pattern.ends_with('/'), kind) {
        (false, _) => Ok((pattern, kind)),
        (true, None | Some(FindEntryKind::Directory)) => Ok((
            directories_matching(&pattern),
            Some(FindEntryKind::Directory),
        )),
        (true, Some(FindEntryKind::File)) => Err(FindError::Search(
            "a pattern that ends in '/' lists directories, but type \"f\" lists only files: \
             drop the '/' or the type"
                .into(),
        )),
    }
}

/// What a pattern ending in '/' asks for, without the '/' and any trailing
/// '.': `./` (or `/`) is every directory, never a search for the name '.'
/// (#2200 review 3).
fn directories_matching(pattern: &str) -> String {
    let mut segments: Vec<&str> = pattern.split('/').collect();
    while let Some(&("" | ".")) = segments.last() {
        segments.pop();
    }
    segments.join("/")
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
