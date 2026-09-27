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
//! NAT64: once per client the adapter looks up `ipv4only.arpa` through the
//! same lookup (RFC 7050), within [`DISCOVERY_TIMEOUT`] and the fetch's
//! deadline. The network-specific prefixes its answers reveal are handed to
//! the domain rule, so an address under one (a literal, a DNS answer or a
//! hop) is judged by the IPv4 address the translator reaches. A lookup
//! that errs or times out means no prefix for that fetch and is asked
//! again; an answer, prefixes or none, is kept.
//!
//! The client and its connection pool are web_fetch's own, not the
//! providers' shared client: a client whose resolver, redirect policy and
//! proxy setting the adapter controls is the secure recipe, and a shared
//! client could not carry it.
//!
//! Redirects are followed here, not by reqwest: exactly the statuses
//! reqwest follows (301, 302, 303, 307, 308), at most [`MAX_REDIRECTS`]
//! times, each hop only when its response carries exactly one valid
//! `Location`, under one deadline for the whole fetch as before. Like
//! reqwest, a hop keeps the credentials the URL's userinfo gave while it
//! stays on the same host and port, and loses them for good once it leaves.
//!
//! The adapter authorizes every URL and every hop itself: it does not rely
//! on the use case's `Allowed` gate, and a `FetchRequest` built any other
//! way is held to the same checks. That, not the type system, is why the
//! gate cannot be widened into a bypass (#1942 ledger row 13).
use crate::application::agent_turn::use_cases::web_fetch::{
    FetchFailure, FetchOutcome, FetchRequest, FetchWebContent, HttpStatus,
};
use crate::domain::nat64_prefix::{DISCOVERY_NAME, Nat64Prefix, discovered_prefixes};
use crate::domain::network_destination::{
    NonPublicAddress, authorize_destination, authorize_under, is_fetchable_name,
};
use std::{future::Future, net::IpAddr, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};

#[path = "web_fetch_failure.rs"]
mod failure_detail;
use failure_detail::Attempt;

const MAX_RAW_BYTES: usize = 5 * 1024 * 1024;
/// One deadline for the whole fetch, every redirect included, as reqwest's
/// request timeout was (#1942 changes only who is reached).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest RFC 7050 NAT64 prefix discovery may take (within the fetch
/// deadline); a slower answer means no prefix for this fetch.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);
/// reqwest's default redirect limit, kept.
const MAX_REDIRECTS: usize = 10;
/// The redirect statuses followed: reqwest's own set, kept.
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];
const USER_AGENT: &str = concat!("quecto/", env!("CARGO_PKG_VERSION"));
/// Appended to a transport failure while proxy variables are set, so a
/// fetch failing on a proxy-only network says why.
const PROXY_HINT: &str = "(web_fetch ignores HTTP(S)_PROXY; see #1942)";
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

    /// The connect timeout this recipe sets, for tests of the paths that
    /// carry it.
    #[cfg(test)]
    pub(crate) fn configured_connect_timeout(&self) -> Option<Duration> {
        self.connect_timeout
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
    /// Whether proxy variables were set (and ignored) when it was built.
    proxies_ignored: bool,
    /// The network-specific NAT64 prefixes, discovered once.
    nat64: Arc<Nat64Discovery>,
}

/// RFC 7050 NAT64 prefix discovery, shared by the adapter and its
/// resolver: `ipv4only.arpa` is looked up through the same lookup as every
/// name, once it answers the prefixes it reveals are kept, and until then
/// (no answer yet, a lookup error or a timeout) there are none.
struct Nat64Discovery {
    lookup: Lookup,
    timeout: Duration,
    found: std::sync::OnceLock<Arc<[Nat64Prefix]>>,
}

impl std::fmt::Debug for Nat64Discovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Nat64Discovery")
            .field("found", &self.found.get())
            .finish()
    }
}

impl Nat64Discovery {
    /// The prefixes discovered so far: none until discovery has answered.
    fn known(&self) -> Arc<[Nat64Prefix]> {
        self.found.get().cloned().unwrap_or_else(|| Arc::from([]))
    }

    /// The discovered prefixes, looking them up first if no lookup has
    /// answered yet, within [`Self::timeout`] and never past `deadline`.
    async fn prefixes(&self, deadline: tokio::time::Instant) -> Arc<[Nat64Prefix]> {
        if let Some(found) = self.found.get() {
            return found.clone();
        }
        let bound = self
            .timeout
            .min(deadline.saturating_duration_since(tokio::time::Instant::now()));
        let answered = tokio::time::timeout(bound, (self.lookup)(DISCOVERY_NAME.to_owned())).await;
        if let Ok(Ok(answers)) = answered {
            let answers: Vec<std::net::Ipv6Addr> = answers
                .iter()
                .filter_map(|answer| match answer.ip() {
                    IpAddr::V6(v6) => Some(v6),
                    IpAddr::V4(_) => None,
                })
                .collect();
            let _ = self.found.set(discovered_prefixes(&answers).into());
        }
        self.known()
    }
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
        Self::with_discovery(recipe, policy, lookup, DISCOVERY_TIMEOUT)
    }

    fn with_discovery(
        recipe: WebFetchClientRecipe,
        policy: DestinationPolicy,
        lookup: Lookup,
        discovery_timeout: Duration,
    ) -> Self {
        static NOTED: std::sync::Once = std::sync::Once::new();
        let note = ignored_proxies_note(|name| std::env::var_os(name).is_some());
        if let Some(note) = &note {
            NOTED.call_once(|| tracing::warn!("{note}"));
        }
        let nat64 = Arc::new(Nat64Discovery {
            lookup,
            timeout: discovery_timeout,
            found: std::sync::OnceLock::new(),
        });
        let resolver = AuthorizingResolver {
            policy: policy.address,
            lookup,
            nat64: nat64.clone(),
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
            proxies_ignored: note.is_some(),
            nat64,
        }
    }

    /// `failure`, with [`PROXY_HINT`] on a transport failure while proxy
    /// variables are set.
    fn hinted(&self, failure: FetchFailure) -> FetchFailure {
        match (failure, self.proxies_ignored) {
            (FetchFailure::Transport(message), true) => {
                FetchFailure::Transport(failure_detail::with_hint(&message, PROXY_HINT))
            }
            (failure, _) => failure,
        }
    }
}

#[cfg(test)]
impl ReqwestFetchWebContent {
    /// Over a default recipe with `policy` and the system resolver.
    fn with_policy(policy: DestinationPolicy) -> Self {
        Self::with_lookup(
            WebFetchClientRecipe::default(),
            policy,
            loopback_test_lookup,
        )
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

/// [`TEST_HOST`] resolves to 127.0.0.1 and [`DISCOVERY_NAME`] to nothing;
/// every other name as in production.
#[cfg(any(test, feature = "test-support"))]
fn loopback_test_lookup(
    host: String,
) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send>> {
    match host.as_str() {
        TEST_HOST => Box::pin(async { Ok(vec![SocketAddr::from(([127, 0, 0, 1], 0))]) }),
        // No NAT64 here, and no query leaves the machine for it.
        DISCOVERY_NAME => Box::pin(async { Ok(Vec::new()) }),
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
fn authorize_url(
    url: &url::Url,
    policy: DestinationPolicy,
    prefixes: &[Nat64Prefix],
) -> Result<(), String> {
    let address = match (url.scheme(), url.host()) {
        ("http" | "https", Some(url::Host::Domain(name))) => return (policy.name)(name),
        ("http" | "https", Some(url::Host::Ipv4(v4))) => IpAddr::V4(v4),
        ("http" | "https", Some(url::Host::Ipv6(v6))) => IpAddr::V6(v6),
        ("http" | "https", None) => return Err("the URL names no host".into()),
        _ => return Err("only http and https are fetched".into()),
    };
    authorize_under(address, prefixes, policy.address).map_err(|refused| refused.to_string())
}

fn authorize_hop(
    url: &url::Url,
    policy: DestinationPolicy,
    prefixes: &[Nat64Prefix],
) -> Result<(), FetchFailure> {
    authorize_url(url, policy, prefixes).map_err(|reason| {
        FetchFailure::Refused(format!(
            "the redirect to {}: {reason}",
            failure_detail::shown(url)
        ))
    })
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
    let refused =
        |why: String| FetchFailure::BadRedirect(format!("the redirect from {base}: {why}"));
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

fn has_userinfo(url: &url::Url) -> bool {
    !url.username().is_empty() || url.password().is_some()
}

/// Whether two URLs name the same host and port, as reqwest judges a
/// cross-host redirect.
fn same_host(a: &url::Url, b: &url::Url) -> bool {
    a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default()
}

/// The URL a hop is requested at: `url` itself, or, when it has no
/// userinfo of its own, `url` with the carried `credentials` (from which
/// reqwest makes the `Authorization` header, as it did on its own hops).
fn with_credentials(url: &url::Url, credentials: Option<&url::Url>) -> url::Url {
    let mut target = url.clone();
    match (credentials, has_userinfo(url)) {
        (Some(carried), false) => {
            let _ = target.set_username(carried.username());
            let _ = target.set_password(carried.password());
            target
        }
        _ => target,
    }
}

/// Resolves a name once and hands the connector only the addresses the
/// policy admits: the connector never resolves again, so the checked
/// answer is the connected one.
struct AuthorizingResolver {
    policy: AddressPolicy,
    lookup: Lookup,
    nat64: Arc<Nat64Discovery>,
}
impl reqwest::dns::Resolve for AuthorizingResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let policy = self.policy;
        let lookup = self.lookup;
        let prefixes = self.nat64.known();
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let answers = lookup(host.clone()).await?;
            let admitted = authorized_answers(&host, answers, policy, &prefixes)?;
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
    prefixes: &[Nat64Prefix],
) -> Result<Vec<SocketAddr>, Refused> {
    if answers.is_empty() {
        return Err(Refused(format!("{host} resolves to no address")));
    }
    let (admitted, refused): (Vec<_>, Vec<_>) = answers
        .into_iter()
        .map(|answer| (answer, authorize_under(answer.ip(), prefixes, policy)))
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

/// A refusal anywhere in `error`'s chain, or else a timeout or a transport
/// failure saying its cause (#2209) on the `attempt` it failed on.
fn failure(error: reqwest::Error, attempt: Attempt<'_>) -> FetchFailure {
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
        // Named by the detail without its userinfo, never by reqwest's
        // message, which names the URL as requested.
        let error = error.without_url();
        FetchFailure::Transport(failure_detail::describe(&error, attempt))
    }
}

impl ReqwestFetchWebContent {
    /// Requests `url`, following authorized redirects, all before one
    /// deadline; the final response is returned unread, with how many
    /// redirects were followed to it.
    async fn final_response(
        &self,
        client: &reqwest::Client,
        url: &url::Url,
        deadline: tokio::time::Instant,
        prefixes: &[Nat64Prefix],
    ) -> Result<(reqwest::Response, usize), FetchFailure> {
        // The first URL's credentials, carried while hops stay on its host.
        let mut credentials = has_userinfo(url).then(|| url.clone());
        let mut current = url.clone();
        let mut previous: Option<url::Url> = None;
        let mut follows = 0;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(FetchFailure::TimedOut);
            }
            let mut request = client
                .get(with_credentials(&current, credentials.as_ref()))
                .timeout(remaining)
                .header(reqwest::header::USER_AGENT, USER_AGENT);
            if let Some(value) = previous.as_ref().and_then(|p| referer(&current, p)) {
                request = request.header(reqwest::header::REFERER, value);
            }
            let response = request.send().await.map_err(|error| {
                // Counted, not compared: a loop may lead back to `url`.
                self.hinted(failure(error, Attempt::after(follows, &current)))
            })?;
            let status = response.status().as_u16();
            let Some(next) = redirect_target(status, response.headers(), &current)? else {
                return Ok((response, follows));
            };
            if follows == MAX_REDIRECTS {
                return Err(FetchFailure::Transport(format!(
                    "too many redirects: {MAX_REDIRECTS} followed, and {} would be one more",
                    failure_detail::shown(&next)
                )));
            }
            authorize_hop(&next, self.policy, prefixes)?;
            credentials = credentials.filter(|_| same_host(&current, &next));
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
            let deadline = tokio::time::Instant::now() + self.timeout;
            let prefixes = self.nat64.prefixes(deadline).await;
            authorize_url(url, self.policy, &prefixes).map_err(FetchFailure::Refused)?;
            let (response, follows) = self
                .final_response(client, url, deadline, &prefixes)
                .await?;
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
            read_body(response, MAX_RAW_BYTES, follows)
                .await
                .map(|body| FetchOutcome::SuccessBody { body, content_type })
        })
    }
}
/// The body of `response`, reached after `follows` redirects, at most
/// `max` bytes.
async fn read_body(
    mut response: reqwest::Response,
    max: usize,
    follows: usize,
) -> Result<Vec<u8>, FetchFailure> {
    let attempted = response.url().clone();
    if let Some(n) = response.content_length() {
        if n as usize > max {
            return Err(FetchFailure::TooLarge {
                actual_bytes: Some(n as usize),
                max_bytes: max,
            });
        }
    }
    let mut out = Vec::with_capacity(max.min(256 * 1024));
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        FetchFailure::Read(failure_detail::describe(
            &e.without_url(),
            Attempt::after(follows, &attempted),
        ))
    })? {
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
#[path = "web_fetch_cause_tests.rs"]
mod cause_tests;
#[cfg(test)]
#[path = "web_fetch_destination_tests.rs"]
mod destination_tests;
#[cfg(test)]
#[path = "web_fetch_lifecycle_tests.rs"]
mod lifecycle_tests;
#[cfg(test)]
#[path = "web_fetch_nat64_tests.rs"]
mod nat64_tests;
#[cfg(test)]
#[path = "web_fetch_reason_tests.rs"]
mod reason_tests;
#[cfg(test)]
#[path = "web_fetch_redirect_tests.rs"]
mod redirect_tests;
#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "web_fetch_transport_tests.rs"]
mod transport_tests;
