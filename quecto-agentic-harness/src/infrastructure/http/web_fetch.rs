use crate::application::agent_turn::use_cases::web_fetch::{
    FetchFailure, FetchOutcome, FetchRequest, FetchWebContent, HttpStatus,
};
use std::{future::Future, pin::Pin, time::Duration};
const MAX_RAW_BYTES: usize = 5 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
#[derive(Clone, Debug)]
pub struct ReqwestFetchWebContent {
    client: reqwest::Client,
}
impl ReqwestFetchWebContent {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}
impl FetchWebContent for ReqwestFetchWebContent {
    fn fetch<'a>(
        &'a self,
        request: &'a FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchOutcome, FetchFailure>> + Send + 'a>> {
        Box::pin(async move {
            let response = self
                .client
                .get(request.url.as_url().clone())
                .timeout(REQUEST_TIMEOUT)
                .header("User-Agent", concat!("quecto/", env!("CARGO_PKG_VERSION")))
                .send()
                .await
                .map_err(|e| {
                    if e.is_timeout() {
                        FetchFailure::TimedOut
                    } else {
                        FetchFailure::Transport(e.to_string())
                    }
                })?;
            if !response.status().is_success() {
                let status = response.status();
                return Ok(FetchOutcome::NonSuccessStatus(HttpStatus::new(
                    status.as_u16(),
                    status.canonical_reason().map(str::to_owned),
                )));
            }
            read_body(response, MAX_RAW_BYTES)
                .await
                .map(FetchOutcome::SuccessBody)
        })
    }
}
async fn read_body(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>, FetchFailure> {
    if let Some(n) = response.content_length() {
        if n as usize > max {
            return Err(FetchFailure::TooLarge {
                actual_bytes: Some(n as usize),
                max_bytes: max,
            });
        }
    }
    let mut out = Vec::with_capacity(max.min(256 * 1024));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| FetchFailure::Read(e.to_string()))?
    {
        out.extend_from_slice(&chunk);
        if out.len() > max {
            return Err(FetchFailure::TooLarge {
                actual_bytes: None,
                max_bytes: max,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
