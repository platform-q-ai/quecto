//! The proxies reqwest takes from the environment (#1942), found the way
//! reqwest finds them (through hyper-util's `Matcher::from_env`): for each
//! of `ALL_PROXY`, `HTTPS_PROXY` and `HTTP_PROXY` the upper-case name is read
//! first and the first one set wins, even when empty; a value hyper-util
//! cannot use configures no proxy; `ALL_PROXY` fills in for an HTTP or HTTPS
//! proxy that is not configured; and under CGI (`REQUEST_METHOD` set) every
//! proxy is off. Platform proxy settings (macOS, Windows) are not read here.
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

    /// Lower-cased host names of the proxies in effect.
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

/// A usable proxy value: the variable it came from and its host.
struct Configured {
    variable: &'static str,
    host: ProxyHost,
}

enum ProxyHost {
    Name(String),
    Literal,
}

pub(crate) fn proxy_environment(lookup: impl Fn(&str) -> Option<String>) -> ProxyEnvironment {
    if lookup("REQUEST_METHOD").is_some() {
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

/// The host of a proxy value hyper-util would use: `http`, `https` or
/// `socks*` (no scheme meaning `http`) with a host.
fn usable_host(value: &str) -> Option<ProxyHost> {
    let url = match value.contains("://") {
        true => url::Url::parse(value),
        false => url::Url::parse(&format!("http://{value}")),
    }
    .ok()?;
    let usable = matches!(
        url.scheme(),
        "http" | "https" | "socks4" | "socks4a" | "socks5" | "socks5h"
    );
    let host = url.host_str().filter(|host| usable && !host.is_empty())?;
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    Some(match bare.parse::<IpAddr>() {
        Ok(_) => ProxyHost::Literal,
        Err(_) => ProxyHost::Name(bare.to_ascii_lowercase()),
    })
}

#[cfg(test)]
#[path = "proxy_env_tests.rs"]
mod tests;
