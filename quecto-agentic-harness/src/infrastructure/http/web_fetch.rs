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
//! because the proxy resolves names and connects on its own. The proxies'
//! own names are resolved here unfiltered (reqwest reaches a proxy through
//! this resolver), and for that reason a URL or hop naming a proxy host is
//! never fetched: the unfiltered answer is reached only by connecting to the
//! proxy itself. Every URL and hop is also held to the courtesy local-name
//! check ([`is_fetchable_name`]).
use crate::application::agent_turn::use_cases::web_fetch::{
    FetchFailure, FetchOutcome, FetchRequest, FetchWebContent, HttpStatus,
};
use crate::domain::network_destination::{
    NonPublicAddress, authorize_destination, is_fetchable_name,
};
use crate::infrastructure::http::proxy_env::{ProxyEnvironment, host_key, process_proxies};
use std::{future::Future, net::IpAddr, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};
const MAX_RAW_BYTES: usize = 5 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// reqwest's default redirect limit, kept (#1942 changes only who is reached).
const MAX_REDIRECTS: usize = 10;

/// Which addresses may be reached: production admits public space only.
type AddressPolicy = fn(IpAddr) -> Result<(), NonPublicAddress>;

/// Which names a URL or hop may give: production admits every name outside
/// the courtesy local-name list.
type NamePolicy = fn(&str) -> Result<(), String>;

/// What a fetch may reach: every address, and every name a URL or hop gives.
#[derive(Clone, Copy, Debug)]
struct DestinationPolicy {
    address: AddressPolicy,
    name: NamePolicy,
}

impl DestinationPolicy {
    const PRODUCTION: Self = Self {
        address: authorize_destination,
        name: admitted_name,
    };
}

/// The production name rule: a fetchable name ([`is_fetchable_name`]).
fn admitted_name(name: &str) -> Result<(), String> {
    match is_fetchable_name(name) {
        true => Ok(()),
        false => Err(format!("{name} is a local name")),
    }
}

/// Resolves a host name to its addresses (the port is the connector's).
type Lookup = fn(String) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send>>;

/// The system resolver, as reqwest's default resolver uses it.
fn system_lookup(
    host: String,
) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send>> {
    Box::pin(async move { Ok(tokio::net::lookup_host((host.as_str(), 0)).await?.collect()) })
}

#[derive(Clone, Debug)]
pub struct ReqwestFetchWebContent {
    /// The client, or why it could not be built: a fetch then fails closed.
    client: Result<reqwest::Client, String>,
    policy: DestinationPolicy,
    /// The configured proxies' own names, never fetch targets.
    proxy_names: Arc<[String]>,
}
impl ReqwestFetchWebContent {
    /// Over the shared client recipe (headers, TLS trust, timeouts), with
    /// destination enforcement laid over it last.
    pub fn new(builder: reqwest::ClientBuilder) -> Self {
        Self::with_environment(builder, DestinationPolicy::PRODUCTION, process_proxies())
    }

    /// Tests' local servers listen on 127.0.0.1 and are named `localhost`:
    /// this adapter admits that one address and that one name beyond
    /// production, and is never compiled into production. It uses no proxy
    /// at all, whatever the process environment says, so no test depends
    /// on it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn allowing_loopback_for_tests(builder: reqwest::ClientBuilder) -> Self {
        Self::with_environment(
            builder.no_proxy(),
            LOOPBACK_FOR_TESTS,
            ProxyEnvironment::default(),
        )
    }

    fn with_environment(
        builder: reqwest::ClientBuilder,
        policy: DestinationPolicy,
        proxies: ProxyEnvironment,
    ) -> Self {
        Self::with_lookup(builder, policy, proxies, system_lookup)
    }

    fn with_lookup(
        builder: reqwest::ClientBuilder,
        policy: DestinationPolicy,
        proxies: ProxyEnvironment,
        lookup: Lookup,
    ) -> Self {
        static WARNED: std::sync::Once = std::sync::Once::new();
        if let Some(warning) = proxies.warning() {
            WARNED.call_once(|| tracing::warn!("{warning}"));
        }
        // Proxies are kept as before (#1942 changes no proxy behaviour
        // without approval). reqwest reaches a proxy through this same
        // resolver, so the proxies' own names are passed unfiltered: an
        // internal proxy (`proxy.corp` on 10.x, `localhost`) keeps working.
        // Through a proxy, the fetched URL's name is resolved by the proxy,
        // not here, so only address literals (the URL's, checked in
        // `fetch`, and each hop's, checked by the redirect policy) are
        // enforced.
        // That exemption is reachable only by connecting to a proxy: a URL
        // or hop may name only an admitted fetch host
        // ([`is_admitted_fetch_host`]), never a proxy's.
        let proxy_names: Arc<[String]> = proxies.proxy_names().into();
        let resolver = AuthorizingResolver {
            policy: policy.address,
            proxy_names: proxy_names.clone(),
            lookup,
        };
        let client = builder
            .dns_resolver(Arc::new(resolver))
            .redirect(authorizing_redirects(policy, proxy_names.clone()))
            .build()
            .map_err(|e| format!("the web-fetch client could not be built: {e}"));
        Self {
            client,
            policy,
            proxy_names,
        }
    }
}

#[cfg(test)]
impl ReqwestFetchWebContent {
    /// Over a default client with no proxy at all, whatever the process
    /// environment says: the adapter's own tests never depend on it.
    fn without_proxies(policy: DestinationPolicy) -> Self {
        Self::with_environment(
            reqwest::Client::builder().no_proxy(),
            policy,
            ProxyEnvironment::default(),
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
const LOOPBACK_FOR_TESTS: DestinationPolicy = DestinationPolicy {
    address: loopback_or_public,
    name: localhost_or_production,
};

#[cfg(any(test, feature = "test-support"))]
fn loopback_or_public(address: IpAddr) -> Result<(), NonPublicAddress> {
    match address {
        IpAddr::V4(v4) if v4 == std::net::Ipv4Addr::LOCALHOST => Ok(()),
        _ => authorize_destination(address),
    }
}

#[cfg(any(test, feature = "test-support"))]
fn localhost_or_production(name: &str) -> Result<(), String> {
    match name {
        "localhost" => Ok(()),
        _ => admitted_name(name),
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

/// Whether `url`'s host is an admitted fetch host: an address literal
/// (judged by the address policy) or a name outside the exception set, the
/// configured proxies' own hosts (compared by [`host_key`]). A proxy name
/// resolves unfiltered so reqwest can reach the proxy; fetching it directly
/// would reach that unfiltered answer.
fn is_admitted_fetch_host(url: &url::Url, proxy_names: &[String]) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => !proxy_names.contains(&host_key(name)),
        Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)) | None => true,
    }
}

/// Why `url` may not be reached, if it may not: its scheme, its address
/// literal, or its name (a proxy's host, or refused by the name policy). A
/// name's addresses are left to [`AuthorizingResolver`], which checks every
/// address it resolves to.
fn authorize_url(
    url: &url::Url,
    policy: DestinationPolicy,
    proxy_names: &[String],
) -> Result<(), String> {
    let address = match (url.scheme(), url.host()) {
        ("http" | "https", Some(url::Host::Domain(name))) => {
            return match is_admitted_fetch_host(url, proxy_names) {
                true => (policy.name)(name),
                false => Err(format!(
                    "{} is a configured proxy host, not a fetch target",
                    host_key(name)
                )),
            };
        }
        ("http" | "https", Some(url::Host::Ipv4(v4))) => IpAddr::V4(v4),
        ("http" | "https", Some(url::Host::Ipv6(v6))) => IpAddr::V6(v6),
        ("http" | "https", None) => return Err("the URL names no host".into()),
        _ => return Err("only http and https are fetched".into()),
    };
    (policy.address)(address).map_err(|refused| refused.to_string())
}

fn authorize_hop(
    url: &url::Url,
    policy: DestinationPolicy,
    proxy_names: &[String],
) -> Result<(), Refused> {
    authorize_url(url, policy, proxy_names)
        .map_err(|reason| Refused(format!("the redirect to {url}: {reason}")))
}

/// reqwest's default redirect handling (the same statuses, at most
/// [`MAX_REDIRECTS`] follows), with every hop authorized before it is
/// followed.
fn authorizing_redirects(
    policy: DestinationPolicy,
    proxy_names: Arc<[String]>,
) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        // `previous` starts with the original URL, as in reqwest's limit.
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        match authorize_hop(attempt.url(), policy, &proxy_names) {
            Ok(()) => attempt.follow(),
            Err(refused) => attempt.error(refused),
        }
    })
}

/// Resolves a name once and hands the connector only the addresses the
/// policy admits: the connector never resolves again, so the checked
/// answer is the connected one.
struct AuthorizingResolver {
    policy: AddressPolicy,
    /// The configured proxies' own names, passed unfiltered.
    proxy_names: Arc<[String]>,
    lookup: Lookup,
}
impl reqwest::dns::Resolve for AuthorizingResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let policy = self.policy;
        let proxy_names = self.proxy_names.clone();
        let lookup = self.lookup;
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let answers = lookup(host.clone()).await?;
            let admitted = admitted_answers(&host, answers, policy, &proxy_names)?;
            assert!(!admitted.is_empty(), "an admitted answer is never empty");
            let addresses: reqwest::dns::Addrs = Box::new(admitted.into_iter());
            Ok(addresses)
        })
    }
}

/// Every answer for a configured proxy's own name (the proxy is the
/// operator's choice, reached as before; a URL or hop never names it, see
/// [`is_admitted_fetch_host`]); otherwise [`authorized_answers`].
fn admitted_answers(
    host: &str,
    answers: Vec<SocketAddr>,
    policy: AddressPolicy,
    proxy_names: &[String],
) -> Result<Vec<SocketAddr>, Refused> {
    let key = host_key(host);
    let is_proxy = proxy_names.contains(&key);
    match is_proxy && !answers.is_empty() {
        true => Ok(answers),
        false => authorized_answers(host, answers, policy),
    }
}

/// The answers `policy` admits, or a refusal naming every refused answer
/// when none is admitted.
fn authorized_answers(
    host: &str,
    answers: Vec<SocketAddr>,
    policy: AddressPolicy,
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
            authorize_url(url, self.policy, &self.proxy_names).map_err(FetchFailure::Refused)?;
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
#[path = "web_fetch_proxy_tests.rs"]
mod proxy_tests;
#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
