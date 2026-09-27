//! The web-fetch port over reqwest, reaching only authorized destinations
//! (#1942).
//!
//! Every address a fetch could reach is checked by one policy before any
//! byte is sent to it: the URL's own address literal, every address a name
//! resolves to (resolved once, here, and only the checked addresses are
//! handed to the connector, so a second, different answer cannot be
//! connected), and every redirect hop before it is followed.
//!
//! A proxy configured by `HTTP(S)_PROXY`/`ALL_PROXY` is honoured as before:
//! through it, only the URL's (and each hop's) literal address is checked,
//! because the proxy resolves names and connects on its own.
use crate::application::agent_turn::use_cases::web_fetch::{
    FetchFailure, FetchOutcome, FetchRequest, FetchWebContent, HttpStatus,
};
use crate::domain::network_destination::{NonPublicAddress, authorize_destination};
use std::{future::Future, net::IpAddr, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};
const MAX_RAW_BYTES: usize = 5 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// reqwest's default redirect limit, kept (#1942 changes only who is reached).
const MAX_REDIRECTS: usize = 10;

/// Which addresses may be reached: production admits public space only.
type DestinationPolicy = fn(IpAddr) -> Result<(), NonPublicAddress>;

#[derive(Clone, Debug)]
pub struct ReqwestFetchWebContent {
    /// The client, or why it could not be built: a fetch then fails closed.
    client: Result<reqwest::Client, String>,
    policy: DestinationPolicy,
}
impl ReqwestFetchWebContent {
    /// Over the shared client recipe (headers, TLS trust, timeouts), with
    /// destination enforcement laid over it last.
    pub fn new(builder: reqwest::ClientBuilder) -> Self {
        Self::with_policy(builder, authorize_destination)
    }

    /// Tests' local servers listen on 127.0.0.1: this adapter admits that one
    /// address beyond public space, and is never compiled into production.
    #[cfg(any(test, feature = "test-support"))]
    pub fn allowing_loopback_for_tests(builder: reqwest::ClientBuilder) -> Self {
        Self::with_policy(builder, loopback_or_public)
    }

    fn with_policy(builder: reqwest::ClientBuilder, policy: DestinationPolicy) -> Self {
        warn_once_when_proxied(|name| std::env::var(name).ok());
        // Proxies are kept (#1942 changes no proxy behaviour without
        // approval). Through a proxy the resolver below is not consulted:
        // the proxy resolves names and connects, so only address literals
        // (the URL's, checked in `fetch`, and each hop's, checked by the
        // redirect policy) are enforced.
        let client = builder
            .dns_resolver(Arc::new(AuthorizingResolver { policy }))
            .redirect(authorizing_redirects(policy))
            .build()
            .map_err(|e| format!("the web-fetch client could not be built: {e}"));
        Self { client, policy }
    }
}

#[cfg(any(test, feature = "test-support"))]
fn loopback_or_public(address: IpAddr) -> Result<(), NonPublicAddress> {
    match address {
        IpAddr::V4(v4) if v4 == std::net::Ipv4Addr::LOCALHOST => Ok(()),
        _ => authorize_destination(address),
    }
}

/// The proxy variable reqwest will use, found the way reqwest (through
/// hyper-util's `Matcher::from_env`) finds it: the first of each upper/lower
/// case pair that is set, in effect when non-empty; `HTTP_PROXY` is ignored
/// under CGI (`REQUEST_METHOD` set). Platform proxy settings (macOS,
/// Windows) are not seen here.
fn proxy_variable_in_effect(lookup: impl Fn(&str) -> Option<String>) -> Option<&'static str> {
    let cgi = lookup("REQUEST_METHOD").is_some();
    let pairs: [(&'static str, &'static str, bool); 3] = [
        ("ALL_PROXY", "all_proxy", true),
        ("HTTPS_PROXY", "https_proxy", true),
        ("HTTP_PROXY", "http_proxy", !cgi),
    ];
    pairs
        .into_iter()
        .filter(|(_, _, honoured)| *honoured)
        .find_map(|(upper, lower, _)| {
            let (name, value) = [upper, lower]
                .into_iter()
                .find_map(|name| lookup(name).map(|value| (name, value)))?;
            (!value.is_empty()).then_some(name)
        })
}

/// What web_fetch says when a proxy variable is in effect.
fn proxy_warning(lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    proxy_variable_in_effect(lookup).map(|name| {
        format!(
            "web_fetch: {name} is set; fetches through an HTTP(S) proxy are checked only on the URL's literal address; the proxy resolves names"
        )
    })
}

/// Logs [`proxy_warning`] the first time a web-fetch adapter is built.
fn warn_once_when_proxied(lookup: impl Fn(&str) -> Option<String>) {
    static WARNED: std::sync::Once = std::sync::Once::new();
    if let Some(warning) = proxy_warning(lookup) {
        WARNED.call_once(|| tracing::warn!("{warning}"));
    }
}

/// A destination refused by the policy, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Refused(String);
impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Refused {}

/// Why `url`'s own address may not be reached, if it may not. A name is
/// left to [`AuthorizingResolver`], which checks every address it resolves
/// to.
fn authorize_url(url: &url::Url, policy: DestinationPolicy) -> Result<(), String> {
    let address = match (url.scheme(), url.host()) {
        ("http" | "https", Some(url::Host::Domain(_))) => return Ok(()),
        ("http" | "https", Some(url::Host::Ipv4(v4))) => IpAddr::V4(v4),
        ("http" | "https", Some(url::Host::Ipv6(v6))) => IpAddr::V6(v6),
        ("http" | "https", None) => return Err("the URL names no host".into()),
        _ => return Err("only http and https are fetched".into()),
    };
    policy(address).map_err(|refused| refused.to_string())
}

fn authorize_hop(url: &url::Url, policy: DestinationPolicy) -> Result<(), Refused> {
    authorize_url(url, policy).map_err(|reason| Refused(format!("the redirect to {url}: {reason}")))
}

/// reqwest's default redirect handling (the same statuses, at most
/// [`MAX_REDIRECTS`] follows), with every hop authorized before it is
/// followed.
fn authorizing_redirects(policy: DestinationPolicy) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        // `previous` starts with the original URL, as in reqwest's limit.
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        match authorize_hop(attempt.url(), policy) {
            Ok(()) => attempt.follow(),
            Err(refused) => attempt.error(refused),
        }
    })
}

/// Resolves a name once and hands the connector only the addresses the
/// policy admits: the connector never resolves again, so the checked
/// answer is the connected one.
struct AuthorizingResolver {
    policy: DestinationPolicy,
}
impl reqwest::dns::Resolve for AuthorizingResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let policy = self.policy;
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let answers: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let admitted = authorized_answers(&host, answers, policy)?;
            assert!(!admitted.is_empty(), "an admitted answer is never empty");
            let addresses: reqwest::dns::Addrs = Box::new(admitted.into_iter());
            Ok(addresses)
        })
    }
}

/// The answers `policy` admits, or a refusal naming every refused answer
/// when none is admitted.
fn authorized_answers(
    host: &str,
    answers: Vec<SocketAddr>,
    policy: DestinationPolicy,
) -> Result<Vec<SocketAddr>, Refused> {
    if answers.is_empty() {
        return Err(Refused(format!("{host} resolves to no address")));
    }
    let (admitted, refused): (Vec<_>, Vec<_>) = answers
        .into_iter()
        .map(|answer| (answer, policy(answer.ip())))
        .partition(|(_, verdict)| verdict.is_ok());
    if admitted.is_empty() {
        let reasons: Vec<String> = refused
            .iter()
            .filter_map(|(_, verdict)| verdict.err().map(|refused| refused.to_string()))
            .collect();
        return Err(Refused(format!(
            "{host} resolves to no public address: {}",
            reasons.join("; ")
        )));
    }
    Ok(admitted.into_iter().map(|(answer, _)| answer).collect())
}

/// A refusal anywhere in `error`'s chain, or else a timeout or transport
/// failure.
fn failure(error: reqwest::Error) -> FetchFailure {
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(current) = cause {
        if let Some(refused) = current.downcast_ref::<Refused>() {
            return FetchFailure::Refused(refused.0.clone());
        }
        cause = current.source();
    }
    if error.is_timeout() {
        FetchFailure::TimedOut
    } else {
        FetchFailure::Transport(error.to_string())
    }
}

impl FetchWebContent for ReqwestFetchWebContent {
    fn fetch<'a>(
        &'a self,
        request: &'a FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchOutcome, FetchFailure>> + Send + 'a>> {
        Box::pin(async move {
            let client = self
                .client
                .as_ref()
                .map_err(|e| FetchFailure::Transport(e.clone()))?;
            let url = request.url.as_url();
            authorize_url(url, self.policy).map_err(FetchFailure::Refused)?;
            let response = client
                .get(url.clone())
                .timeout(REQUEST_TIMEOUT)
                .header("User-Agent", concat!("quecto/", env!("CARGO_PKG_VERSION")))
                .send()
                .await
                .map_err(failure)?;
            if !response.status().is_success() {
                let status = response.status();
                return Ok(FetchOutcome::NonSuccessStatus(HttpStatus::new(
                    status.as_u16(),
                    status.canonical_reason().map(str::to_owned),
                )));
            }
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            read_body(response, MAX_RAW_BYTES)
                .await
                .map(|body| FetchOutcome::SuccessBody { body, content_type })
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
#[path = "web_fetch_destination_tests.rs"]
mod destination_tests;
#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
