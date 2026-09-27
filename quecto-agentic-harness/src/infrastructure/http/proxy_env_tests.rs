use super::*;

fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |name: &str| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_string())
    }
}

fn names(pairs: &'static [(&'static str, &'static str)]) -> Vec<String> {
    proxy_environment(env(pairs)).proxy_names().to_vec()
}

fn variables(pairs: &'static [(&'static str, &'static str)]) -> Vec<&'static str> {
    proxy_environment(env(pairs)).variables().to_vec()
}

#[test]
fn no_proxy_variable_means_no_proxy() {
    assert_eq!(proxy_environment(env(&[])), ProxyEnvironment::default());
    // NO_PROXY alone configures no proxy.
    assert_eq!(variables(&[("NO_PROXY", "*")]), Vec::<&str>::new());
}

#[test]
fn each_variable_is_read_upper_case_first_and_empty_means_none() {
    assert_eq!(variables(&[("https_proxy", "http://p:1")]), ["https_proxy"]);
    assert_eq!(variables(&[("HTTP_PROXY", "http://p:1")]), ["HTTP_PROXY"]);
    assert_eq!(variables(&[("ALL_PROXY", "socks5://p:1")]), ["ALL_PROXY"]);
    // The upper-case name wins even when empty, as in hyper-util: no proxy.
    assert_eq!(
        variables(&[("HTTPS_PROXY", ""), ("https_proxy", "http://p:1")]),
        Vec::<&str>::new()
    );
    assert_eq!(
        variables(&[("HTTPS_PROXY", "http://a:1"), ("https_proxy", "http://b:1")]),
        ["HTTPS_PROXY"]
    );
    assert_eq!(
        names(&[("HTTPS_PROXY", "http://a:1"), ("https_proxy", "http://b:1")]),
        ["a"]
    );
}

#[test]
fn under_cgi_every_proxy_is_off() {
    for pairs in [
        &[("REQUEST_METHOD", "GET"), ("HTTP_PROXY", "http://p:1")][..],
        &[("REQUEST_METHOD", "GET"), ("HTTPS_PROXY", "http://p:1")][..],
        &[("REQUEST_METHOD", "GET"), ("ALL_PROXY", "http://p:1")][..],
    ] {
        let lookup = move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        };
        assert_eq!(
            proxy_environment(lookup),
            ProxyEnvironment::default(),
            "{pairs:?}"
        );
    }
}

#[test]
fn proxy_names_are_the_hosts_of_the_proxies_in_effect() {
    assert_eq!(
        names(&[("HTTPS_PROXY", "http://Proxy.Corp:3128")]),
        ["proxy.corp"]
    );
    // No scheme means http, as in hyper-util.
    assert_eq!(names(&[("http_proxy", "localhost:3128")]), ["localhost"]);
    assert_eq!(
        names(&[("HTTP_PROXY", "http://user:pw@squid.internal:3128/")]),
        ["squid.internal"]
    );
    // ALL_PROXY fills in for whichever of HTTP and HTTPS is not set.
    assert_eq!(
        names(&[
            ("ALL_PROXY", "socks5h://all.proxy:1080"),
            ("HTTPS_PROXY", "http://tls.proxy:1")
        ]),
        ["all.proxy", "tls.proxy"]
    );
    assert_eq!(names(&[("ALL_PROXY", "http://one:1")]), ["one"]);
    // An address literal needs no name exemption.
    assert_eq!(
        names(&[("HTTPS_PROXY", "http://10.0.0.1:3128")]),
        Vec::<String>::new()
    );
    assert_eq!(
        names(&[("HTTPS_PROXY", "http://[fd00::1]:3128")]),
        Vec::<String>::new()
    );
    // A value hyper-util cannot use configures no proxy.
    assert_eq!(
        variables(&[("HTTPS_PROXY", "ftp://p:21")]),
        Vec::<&str>::new()
    );
    assert_eq!(variables(&[("HTTPS_PROXY", "http://")]), Vec::<&str>::new());
}

#[test]
fn the_warning_names_the_variables_and_what_they_bypass() {
    assert_eq!(proxy_environment(env(&[])).warning(), None);
    assert_eq!(
        proxy_environment(env(&[("http_proxy", "http://p:1")]))
            .warning()
            .as_deref(),
        Some(
            "web_fetch: http_proxy is set; fetches through an HTTP(S) proxy are checked only on the URL's literal address; the proxy resolves names"
        )
    );
    assert_eq!(
        proxy_environment(env(&[
            ("HTTP_PROXY", "http://p:1"),
            ("HTTPS_PROXY", "http://q:1")
        ]))
        .warning()
        .as_deref(),
        Some(
            "web_fetch: HTTP_PROXY, HTTPS_PROXY is set; fetches through an HTTP(S) proxy are checked only on the URL's literal address; the proxy resolves names"
        )
    );
}
