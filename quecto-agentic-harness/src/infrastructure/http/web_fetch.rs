//! The web-fetch port over reqwest, reaching only authorized destinations
//! (#1942).
//!
//! Every address a fetch could reach is checked by one policy before any
//! byte is sent to it: the URL's own address literal, every address a name
//! resolves to (resolved once, here, and only the checked addresses are
//! handed to the connector, so a second, different answer cannot be
//! connected), and every redirect hop before it is requested.
//!
//! The client is built here, and only here, from an owned
//! [`WebFetchClientRecipe`]: no caller hands in a `reqwest::ClientBuilder`,
//! so no caller can bring a proxy, a Unix socket or any other transport
//! that would skip the resolver. The client never uses a proxy: `no_proxy`
//! is applied last, after every recipe setting, and `HTTP(S)_PROXY` /
//! `ALL_PROXY` are ignored (a note is logged once when they are set).
//!
//! Redirects are followed here, not by reqwest: exactly the statuses
//! reqwest follows (301, 302, 303, 307, 308), at most [`MAX_REDIRECTS`]
//! times, each hop only when its response carries exactly one valid
//! `Location`, under one deadline for the whole fetch as before.
use crate::application::agent_turn::use_cases::web_fetch::{
    FetchFailure, FetchOutcome, FetchRequest, FetchWebContent, HttpStatus,
};
use crate::domain::network_destination::{
    NonPublicAddress, authorize_destination, is_fetchable_name,
};
use std::{future::Future, net::IpAddr, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};
const MAX_RAW_BYTES: usize = 5 * 1024 * 1024;
/// One deadline for the whole fetch, every redirect included, as reqwest's
/// request timeout was (#1942 changes only who is reached).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// reqwest's default redirect limit, kept.
const MAX_REDIRECTS: usize = 10;
/// The redirect statuses followed: reqwest's own set, kept.
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];
const USER_AGENT: &str = concat!("quecto/", env!("CARGO_PKG_VERSION"));
/// The proxy variables reqwest would read, all ignored here.
const PROXY_VARIABLES: [&str; 6] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
];

/// Everything web_fetch's client is built from: the connect timeout and
/// any extra trusted root certificates. Nothing else can be set, so no
/// transport other than a direct TCP (and TLS) connection to an authorized
/// address can be configured.
#[derive(Clone, Debug, Default)]
pub struct WebFetchClientRecipe {
    connect_timeout: Option<Duration>,
    root_certificates: Vec<reqwest::Certificate>,
}

impl WebFetchClientRecipe {
    /// The connection handshake's own limit (within the fetch's deadline).
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// Trust `certificate` as a root, beside the built-in roots.
    pub fn add_root_certificate(mut self, certificate: reqwest::Certificate) -> Self {
        self.root_certificates.push(certificate);
        self
    }

    /// The recipe's settings on a fresh builder; the adapter adds its own
    /// enforcement over them.
    fn builder(&self) -> reqwest::ClientBuilder {
        let builder = reqwest::Client::builder();
        let builder = match self.connect_timeout {
            Some(timeout) => builder.connect_timeout(timeout),
            None => builder,
        };
        self.root_certificates
            .iter()
            .cloned()
            .fold(builder, reqwest::ClientBuilder::add_root_certificate)
    }
}

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
    /// The whole fetch's deadline, redirects included.
    timeout: Duration,
}
impl ReqwestFetchWebContent {
    /// Over the production recipe, with destination enforcement laid over
    /// it last.
    pub fn new(recipe: WebFetchClientRecipe) -> Self {
        Self::with_lookup(recipe, DestinationPolicy::PRODUCTION, system_lookup)
    }

    /// Tests' local servers listen on 127.0.0.1 and are named `localhost`
    /// or [`TEST_HOST`]: this adapter admits that one address and the name
    /// `localhost` beyond production, answers [`TEST_HOST`] with 127.0.0.1
    /// (through the same checks), and is never compiled into production.
    #[cfg(any(test, feature = "test-support"))]
    pub fn allowing_loopback_for_tests(recipe: WebFetchClientRecipe) -> Self {
        Self::with_lookup(recipe, LOOPBACK_FOR_TESTS, loopback_test_lookup)
    }

    fn with_lookup(
        recipe: WebFetchClientRecipe,
        policy: DestinationPolicy,
        lookup: Lookup,
    ) -> Self {
        static NOTED: std::sync::Once = std::sync::Once::new();
        if let Some(note) = ignored_proxies_note(|name| std::env::var_os(name).is_some()) {
            NOTED.call_once(|| tracing::info!("{note}"));
        }
        let resolver = AuthorizingResolver {
            policy: policy.address,
            lookup,
        };
        // `no_proxy` is the last setting before `build`: nothing after it
        // can bring a proxy back, and the recipe cannot set one.
        let client = recipe
            .builder()
            .dns_resolver(Arc::new(resolver))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|e| format!("the web-fetch client could not be built: {e}"));
        Self {
            client,
            policy,
            timeout: REQUEST_TIMEOUT,
        }
    }
}

#[cfg(test)]
impl ReqwestFetchWebContent {
    /// Over a default recipe with `policy` and the system resolver.
    fn with_policy(policy: DestinationPolicy) -> Self {
        Self::with_lookup(WebFetchClientRecipe::default(), policy, system_lookup)
    }

    /// The same adapter with a shorter whole-fetch deadline.
    fn with_timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }
}

/// The note logged once when proxy variables are set: web_fetch ignores
/// them, so every destination is checked.
fn ignored_proxies_note(is_set: impl Fn(&str) -> bool) -> Option<String> {
    let set: Vec<&str> = PROXY_VARIABLES
        .into_iter()
        .filter(|name| is_set(name))
        .collect();
    (!set.is_empty()).then(|| {
        format!(
            "web_fetch connects directly and ignores HTTP(S)_PROXY/ALL_PROXY so every destination is checked (#1942); set: {}",
            set.join(", ")
        )
    })
}

/// A name tests use for their local servers where `localhost` would be
/// refused up front by the use case.
#[cfg(any(test, feature = "test-support"))]
pub const TEST_HOST: &str = "web-fetch.test";

/// [`TEST_HOST`] resolves to 127.0.0.1; every other name as in production.
#[cfg(any(test, feature = "test-support"))]
fn loopback_test_lookup(
    host: String,
) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send>> {
    match host.as_str() {
        TEST_HOST => Box::pin(async { Ok(vec![SocketAddr::from(([127, 0, 0, 1], 0))]) }),
        _ => system_lookup(host),
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

/// Why `url` may not be reached, if it may not: its scheme, its address
/// literal, or its name (refused by the name policy). A name's addresses
/// are left to [`AuthorizingResolver`], which checks every address it
/// resolves to.
fn authorize_url(url: &url::Url, policy: DestinationPolicy) -> Result<(), String> {
    let address = match (url.scheme(), url.host()) {
        ("http" | "https", Some(url::Host::Domain(name))) => return (policy.name)(name),
        ("http" | "https", Some(url::Host::Ipv4(v4))) => IpAddr::V4(v4),
        ("http" | "https", Some(url::Host::Ipv6(v6))) => IpAddr::V6(v6),
        ("http" | "https", None) => return Err("the URL names no host".into()),
        _ => return Err("only http and https are fetched".into()),
    };
    (policy.address)(address).map_err(|refused| refused.to_string())
}

fn authorize_hop(url: &url::Url, policy: DestinationPolicy) -> Result<(), FetchFailure> {
    authorize_url(url, policy)
        .map_err(|reason| FetchFailure::Refused(format!("the redirect to {url}: {reason}")))
}

/// Where a response redirects to, if it is a redirect: only one of
/// [`REDIRECT_STATUSES`] with exactly one `Location` that is a non-empty,
/// valid URL (relative to `base`) is followed; any other redirect fails
/// closed before anything is sent to its target.
fn redirect_target(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    base: &url::Url,
) -> Result<Option<url::Url>, FetchFailure> {
    match REDIRECT_STATUSES.contains(&status) {
        true => single_location(headers, base).map(Some),
        false => Ok(None),
    }
}

/// The one `Location` of a redirect from `base`, resolved against it, or a
/// refusal when there is none, more than one, or one that is empty or not
/// a valid URL.
fn single_location(
    headers: &reqwest::header::HeaderMap,
    base: &url::Url,
) -> Result<url::Url, FetchFailure> {
    let locations: Vec<_> = headers.get_all(reqwest::header::LOCATION).iter().collect();
    let refused = |why: String| FetchFailure::Refused(format!("the redirect from {base}: {why}"));
    let [location] = locations.as_slice() else {
        return Err(refused(format!(
            "{} Location headers, not exactly one",
            locations.len()
        )));
    };
    location
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .and_then(|text| base.join(text).ok())
        .ok_or_else(|| refused("its Location is not a valid URL".into()))
}

/// The `Referer` reqwest sends on a followed redirect: the previous URL
/// without credentials or fragment, and none from https to http.
fn referer(next: &url::Url, previous: &url::Url) -> Option<reqwest::header::HeaderValue> {
    match (previous.scheme(), next.scheme()) {
        ("https", "http") => None,
        _ => {
            let mut referer = previous.clone();
            let _ = referer.set_username("");
            let _ = referer.set_password(None);
            referer.set_fragment(None);
            referer.as_str().parse().ok()
        }
    }
}

/// Resolves a name once and hands the connector only the addresses the
/// policy admits: the connector never resolves again, so the checked
/// answer is the connected one.
struct AuthorizingResolver {
    policy: AddressPolicy,
    lookup: Lookup,
}
impl reqwest::dns::Resolve for AuthorizingResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let policy = self.policy;
        let lookup = self.lookup;
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let answers = lookup(host.clone()).await?;
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

impl ReqwestFetchWebContent {
    /// Requests `url`, following authorized redirects, all before one
    /// deadline; the final response is returned unread.
    async fn final_response(
        &self,
        client: &reqwest::Client,
        url: &url::Url,
    ) -> Result<reqwest::Response, FetchFailure> {
        let deadline = tokio::time::Instant::now() + self.timeout;
        let mut current = url.clone();
        let mut previous: Option<url::Url> = None;
        let mut follows = 0;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(FetchFailure::TimedOut);
            }
            let mut request = client
                .get(current.clone())
                .timeout(remaining)
                .header(reqwest::header::USER_AGENT, USER_AGENT);
            if let Some(value) = previous.as_ref().and_then(|p| referer(&current, p)) {
                request = request.header(reqwest::header::REFERER, value);
            }
            let response = request.send().await.map_err(failure)?;
            let status = response.status().as_u16();
            let Some(next) = redirect_target(status, response.headers(), &current)? else {
                return Ok(response);
            };
            if follows == MAX_REDIRECTS {
                return Err(FetchFailure::Transport(format!(
                    "too many redirects: the redirect to {next} would be follow {}",
                    follows + 1
                )));
            }
            authorize_hop(&next, self.policy)?;
            follows += 1;
            previous = Some(std::mem::replace(&mut current, next));
        }
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
            let response = self.final_response(client, url).await?;
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
#[path = "web_fetch_redirect_tests.rs"]
mod redirect_tests;
#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "web_fetch_transport_tests.rs"]
mod transport_tests;
