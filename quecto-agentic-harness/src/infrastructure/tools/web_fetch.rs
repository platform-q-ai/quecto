// Web fetch tool: fetch a URL and return its content as text.
//
// Destination authorization is owned by the application policy. This adapter
// owns only URL parsing, DNS, redirect, connection, HTTP, and body mechanisms.

use std::collections::BTreeSet;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::application::agent_turn::use_cases::web_fetch::{
    FetchWebContent, FetchWebContentError, FetchWebContentRequest, FetchedWebContent,
    WebDestinationAuthorization, WebDestinationPolicy, WebDestinationTarget,
};

/// Maximum raw download size before text extraction (5 MB).
const MAX_RAW_BYTES: usize = 5 * 1024 * 1024;
/// Default request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Reqwest's historic default is ten followed redirects.
const MAX_REDIRECTS: usize = 10;

type ResolutionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, FetchWebContentError>> + Send + 'a>>;

/// Fetch-controlled DNS seam. Implementations return candidates once; the
/// adapter authorizes that exact set and pins it into the connector.
trait DestinationResolver: Send + Sync {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolutionFuture<'a>;
}

#[derive(Debug, Default)]
struct SystemDestinationResolver;

impl DestinationResolver for SystemDestinationResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolutionFuture<'a> {
        Box::pin(async move {
            tokio::net::lookup_host((host, port))
                .await
                .map(|addresses| addresses.collect())
                .map_err(|error| FetchWebContentError::Resolution(error.to_string()))
        })
    }
}

/// Reqwest implementation of the application content-fetching port.
pub struct ReqwestFetchWebContent {
    destination_policy: WebDestinationPolicy,
    resolver: Arc<dyn DestinationResolver>,
}

impl std::fmt::Debug for ReqwestFetchWebContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReqwestFetchWebContent")
            .field("destination_policy", &self.destination_policy)
            .finish_non_exhaustive()
    }
}

impl ReqwestFetchWebContent {
    /// Create with the output cap. A dedicated client is deliberately built
    /// for each authorized hop so automatic redirects and a second DNS lookup
    /// can never bypass the destination decision.
    pub fn new() -> Self {
        Self {
            destination_policy: WebDestinationPolicy::new(),
            resolver: Arc::new(SystemDestinationResolver),
        }
    }

    /// Permit one exact local destination for deterministic tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_allowed_host(host_port: &str) -> Self {
        let (host, port) = parse_test_destination(host_port)
            .expect("test destination must be one exact host:port pair");
        Self {
            destination_policy: WebDestinationPolicy::with_test_destination(host, port),
            resolver: Arc::new(SystemDestinationResolver),
        }
    }

    #[cfg(test)]
    fn with_test_dependencies(
        destination_policy: WebDestinationPolicy,
        resolver: Arc<dyn DestinationResolver>,
    ) -> Self {
        Self {
            destination_policy,
            resolver,
        }
    }

    async fn fetch_hop(&self, url: &reqwest::Url) -> Result<reqwest::Response, FetchHopError> {
        let host = url.host_str().ok_or(FetchHopError::Denied)?;
        let port = url.port_or_known_default().ok_or(FetchHopError::Denied)?;
        let literal_candidate = normalized_ip_literal(host);
        let target = WebDestinationTarget::new(url.scheme(), host, port, literal_candidate);
        if self.destination_policy.authorize(&target) != WebDestinationAuthorization::Allowed {
            return Err(FetchHopError::Denied);
        }

        let authorized_addresses = if let Some(ip) = literal_candidate {
            vec![SocketAddr::new(ip, port)]
        } else {
            let resolved = self
                .resolver
                .resolve(host, port)
                .await
                .map_err(FetchHopError::Mechanical)?;
            authorized_candidates(&self.destination_policy, url, resolved)
        };
        if authorized_addresses.is_empty() {
            return Err(FetchHopError::Denied);
        }
        debug_assert!(
            authorized_addresses.iter().all(|address| {
                let target =
                    WebDestinationTarget::new(url.scheme(), host, port, Some(address.ip()));
                self.destination_policy.authorize(&target) == WebDestinationAuthorization::Allowed
            }),
            "every connector candidate must have explicit authorization"
        );

        // No proxy, no automatic redirect, and no resolver fallback. The only
        // connector candidates are the exact addresses authorized above.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(host, &authorized_addresses)
            .build()
            .map_err(|error| {
                FetchHopError::Mechanical(FetchWebContentError::Transport(error.to_string()))
            })?;
        client
            .get(url.clone())
            .timeout(REQUEST_TIMEOUT)
            .header("User-Agent", concat!("quecto/", env!("CARGO_PKG_VERSION")))
            .send()
            .await
            .map_err(|error| {
                FetchHopError::Mechanical(if error.is_timeout() {
                    FetchWebContentError::TimedOut
                } else {
                    FetchWebContentError::Transport(error.to_string())
                })
            })
    }
}

#[derive(Debug)]
enum FetchHopError {
    Denied,
    Mechanical(FetchWebContentError),
}

fn authorized_candidates(
    policy: &WebDestinationPolicy,
    url: &reqwest::Url,
    candidates: Vec<SocketAddr>,
) -> Vec<SocketAddr> {
    let Some(host) = url.host_str() else {
        return Vec::new();
    };
    let Some(port) = url.port_or_known_default() else {
        return Vec::new();
    };
    candidates
        .into_iter()
        .map(|candidate| SocketAddr::new(candidate.ip(), port))
        .filter(|candidate| {
            let target = WebDestinationTarget::new(url.scheme(), host, port, Some(candidate.ip()));
            policy.authorize(&target) == WebDestinationAuthorization::Allowed
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalized_ip_literal(host: &str) -> Option<IpAddr> {
    host.strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host)
        .parse()
        .ok()
}

#[cfg(any(test, feature = "test-support"))]
fn parse_test_destination(host_port: &str) -> Option<(String, u16)> {
    if let Ok(address) = host_port.parse::<SocketAddr>() {
        return Some((address.ip().to_string(), address.port())).filter(|(_, port)| *port > 0);
    }
    let (host, port) = host_port.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    (!host.is_empty() && port > 0).then(|| (host.to_owned(), port))
}

impl FetchWebContent for ReqwestFetchWebContent {
    fn fetch(
        &self,
        request: FetchWebContentRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<FetchedWebContent, FetchWebContentError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            let mut current_url = reqwest::Url::parse(&request.url)
                .map_err(|error| FetchWebContentError::InvalidUrl(error.to_string()))?;
            let mut redirects = 0_usize;
            let response = loop {
                let response = self
                    .fetch_hop(&current_url)
                    .await
                    .map_err(|error| match error {
                        FetchHopError::Denied => FetchWebContentError::DestinationDenied(
                            current_url.host_str().unwrap_or("unknown").to_owned(),
                        ),
                        FetchHopError::Mechanical(error) => error,
                    })?;
                if !response.status().is_redirection() {
                    break response;
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .ok_or_else(|| {
                        FetchWebContentError::Transport("redirect missing Location header".into())
                    })?
                    .to_str()
                    .map_err(|error| FetchWebContentError::Transport(error.to_string()))?;
                current_url = current_url
                    .join(location)
                    .map_err(|error| FetchWebContentError::InvalidUrl(error.to_string()))?;
                redirects += 1;
                if redirects > MAX_REDIRECTS {
                    return Err(FetchWebContentError::RedirectLimitExceeded);
                }
            };
            let status = response.status().as_u16();
            let body = read_body_capped(response, MAX_RAW_BYTES).await?;
            Ok(FetchedWebContent { status, body })
        })
    }
}

impl Default for ReqwestFetchWebContent {
    fn default() -> Self {
        Self::new()
    }
}

/// Read response body up to `max_bytes` using streaming chunks.
///
/// Aborts mid-stream if the body exceeds the cap, avoiding OOM from
/// servers that send large bodies without a Content-Length header.
async fn read_body_capped(
    mut resp: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, FetchWebContentError> {
    // Pre-flight: reject if Content-Length is known and too large
    if let Some(len) = resp.content_length() {
        if len as usize > max_bytes {
            return Err(FetchWebContentError::ResponseTooLarge {
                actual_bytes: usize::try_from(len).ok(),
                max_bytes,
            });
        }
    }

    let mut buf = Vec::with_capacity(max_bytes.min(256 * 1024));
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| FetchWebContentError::Read(e.to_string()))?
    {
        buf.extend_from_slice(&chunk);
        if buf.len() > max_bytes {
            return Err(FetchWebContentError::ResponseTooLarge {
                actual_bytes: None,
                max_bytes,
            });
        }
    }
    Ok(buf)
}

#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
