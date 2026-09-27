//! The proxies reqwest takes from the environment (#1942), found the way
//! reqwest finds them: through hyper-util's own `Matcher`, so every value is
//! parsed by the parser reqwest uses (a value it cannot use configures no
//! proxy here either). For each of `ALL_PROXY`, `HTTPS_PROXY` and
//! `HTTP_PROXY` the upper-case name is read first and the first one set
//! wins, even when empty; `ALL_PROXY` fills in for an HTTP or HTTPS proxy
//! that is not configured; and under CGI (`REQUEST_METHOD` present, set to
//! anything) every proxy is off. Platform proxy settings (macOS, Windows)
//! are not read here, nor by reqwest without its `system-proxy` feature.
use hyper_util::client::proxy::matcher::Matcher;
use std::net::IpAddr;

/// The proxy variables in effect and the proxies' host names (address
/// literals are left out: they are never resolved).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProxyEnvironment {
    variables: Vec<&'static str>,
    proxy_names: Vec<String>,
}

impl ProxyEnvironment {
    #[cfg(test)]
    pub(crate) fn variables(&self) -> &[&'static str] {
        &self.variables
    }

    /// Host names of the proxies in effect, lower-cased with trailing dots
    /// removed ([`host_key`]).
    pub(crate) fn proxy_names(&self) -> &[String] {
        &self.proxy_names
    }

    /// What web_fetch logs when a proxy is in effect.
    pub(crate) fn warning(&self) -> Option<String> {
        (!self.variables.is_empty()).then(|| {
            format!(
                "web_fetch: {} is set; fetches through an HTTP(S) proxy are checked only on the URL's literal address; the proxy resolves names",
                self.variables.join(", ")
            )
        })
    }
}

/// How two spellings of a host name are compared: case-insensitively and
/// without trailing dots.
pub(crate) fn host_key(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// The proxies this process's environment configures for reqwest.
pub(crate) fn process_proxies() -> ProxyEnvironment {
    // hyper-util tests presence (`var_os`), so a non-UTF-8 value counts,
    // and reads each proxy variable with `var` (a non-UTF-8 value is unset).
    proxy_environment(std::env::var_os("REQUEST_METHOD").is_some(), |name| {
        std::env::var(name).ok()
    })
}

/// A usable proxy value: the variable it came from and its host.
struct Configured {
    variable: &'static str,
    host: ProxyHost,
}

enum ProxyHost {
    Name(String),
    Literal,
}

/// The proxy environment `lookup` describes; `is_cgi` turns every proxy off.
pub(crate) fn proxy_environment(
    is_cgi: bool,
    lookup: impl Fn(&str) -> Option<String>,
) -> ProxyEnvironment {
    if is_cgi {
        return ProxyEnvironment::default();
    }
    let read = |upper: &'static str, lower: &'static str| {
        [upper, lower]
            .into_iter()
            .find_map(|variable| lookup(variable).map(|value| (variable, value)))
            .and_then(|(variable, value)| {
                usable_host(&value).map(|host| Configured { variable, host })
            })
    };
    let http = read("HTTP_PROXY", "http_proxy").or_else(|| read("ALL_PROXY", "all_proxy"));
    let https = read("HTTPS_PROXY", "https_proxy").or_else(|| read("ALL_PROXY", "all_proxy"));
    let mut environment = ProxyEnvironment::default();
    for configured in [http, https].into_iter().flatten() {
        if !environment.variables.contains(&configured.variable) {
            environment.variables.push(configured.variable);
        }
        if let ProxyHost::Name(name) = configured.host {
            if !environment.proxy_names.contains(&name) {
                environment.proxy_names.push(name);
            }
        }
    }
    environment
}

/// The host of the proxy hyper-util makes of `value`, if it makes one: the
/// value is handed to hyper-util's matcher and the proxy it would connect
/// to is read back, so no second parser can disagree with it.
fn usable_host(value: &str) -> Option<ProxyHost> {
    let probe = http::Uri::from_static("http://proxy-probe.invalid/");
    let intercept = Matcher::builder().http(value).build().intercept(&probe)?;
    let host = intercept.uri().host()?;
    assert!(
        !host.is_empty(),
        "hyper-util builds a proxy URI with a host"
    );
    // As hyper's connector sees it: brackets stripped, and only what
    // `IpAddr` parses strictly is an address; anything else (`127.1`,
    // `fe80::1%25eth0`) goes to the resolver as a name.
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    Some(match bare.parse::<IpAddr>() {
        Ok(_) => ProxyHost::Literal,
        Err(_) => ProxyHost::Name(host_key(bare)),
    })
}

#[cfg(test)]
#[path = "proxy_env_tests.rs"]
mod tests;
