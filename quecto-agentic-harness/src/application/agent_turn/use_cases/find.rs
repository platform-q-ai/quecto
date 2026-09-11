//! Typed invocation of path discovery; delivery and filesystem details stay outside.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct FindRequest {
    pub pattern: String,
    pub path: String,
    pub limit: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindPathsRequest {
    pub pattern: String,
    pub path: String,
    pub limit: usize,
}

/// Entries are complete, root-relative paths in backend order, never raw fragments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindOutput {
    pub entries: Vec<String>,
    /// Count-limit heuristic, not evidence that more matches exist.
    pub result_limit_reached: bool,
    pub incomplete: bool,
    pub diagnostic: Option<String>,
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
        let output = self
            .paths
            .find(FindPathsRequest {
                pattern: request.pattern,
                path: request.path,
                limit,
            })
            .await?;
        Ok(FindResult { output, limit })
    }
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
