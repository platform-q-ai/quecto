use super::*;

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name: &str| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_string())
    }
}

fn names(pairs: &[(&str, &str)]) -> Vec<String> {
    proxy_environment(false, env(pairs)).proxy_names().to_vec()
}

fn variables(pairs: &[(&str, &str)]) -> Vec<&'static str> {
    proxy_environment(false, env(pairs)).variables().to_vec()
}

#[test]
fn no_proxy_variable_means_no_proxy() {
    assert_eq!(
        proxy_environment(false, env(&[])),
        ProxyEnvironment::default()
    );
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
        &[("HTTP_PROXY", "http://p:1")][..],
        &[("HTTPS_PROXY", "http://p:1")][..],
        &[("ALL_PROXY", "http://p:1")][..],
    ] {
        assert_eq!(
            proxy_environment(true, env(pairs)),
            ProxyEnvironment::default(),
            "{pairs:?}"
        );
        assert_ne!(
            proxy_environment(false, env(pairs)),
            ProxyEnvironment::default()
        );
    }
}

/// #1942 second review: every value is parsed by hyper-util itself, so a
/// value it makes no proxy of is exempted by no name here.
#[test]
fn a_value_hyper_util_cannot_use_exempts_no_name() {
    for value in [
        "http://p:1 ",
        "http://p:1\t",
        "http://p:1\r",
        " http://p:1",
        "http://%6c%6fcalhost:1",
        "//p:1",
        "http://b\u{fc}cher.example:1",
        "ftp://p:21",
        "http://",
        "",
    ] {
        let pairs = [("HTTPS_PROXY", value)];
        assert_eq!(variables(&pairs), Vec::<&str>::new(), "{value:?}");
        assert_eq!(names(&pairs), Vec::<String>::new(), "{value:?}");
    }
}

/// Hosts hyper's connector does not parse as an address (it parses only a
/// strict dotted quad or IPv6 address) reach the resolver: they are names.
#[test]
fn a_host_the_connector_resolves_is_a_name() {
    for (value, name) in [
        ("http://127.1:3128", "127.1"),
        ("http://0x7f.1:3128", "0x7f.1"),
        ("http://2130706433:3128", "2130706433"),
        ("http://[fe80::1%25eth0]:3128", "fe80::1%25eth0"),
        ("http://p:99999", "p"),
        ("HTTP://Proxy.Corp.:1", "proxy.corp"),
        (
            "http://user:pw@squid.internal:3128/path?q",
            "squid.internal",
        ),
    ] {
        assert_eq!(names(&[("HTTPS_PROXY", value)]), [name], "{value:?}");
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
    assert_eq!(
        names(&[
            ("HTTP_PROXY", "http://plain.proxy:1"),
            ("ALL_PROXY", "http://all.proxy:1")
        ]),
        ["plain.proxy", "all.proxy"],
        "HTTPS falls back to ALL_PROXY too"
    );
    // A socks URL keeps its host's case in `url`: names are lower-cased.
    assert_eq!(
        names(&[("ALL_PROXY", "socks5h://Proxy.Corp:1080")]),
        ["proxy.corp"]
    );
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
    assert_eq!(proxy_environment(false, env(&[])).warning(), None);
    assert_eq!(
        proxy_environment(false, env(&[("http_proxy", "http://p:1")]))
            .warning()
            .as_deref(),
        Some(
            "web_fetch: http_proxy is set; fetches through an HTTP(S) proxy are checked only on the URL's literal address; the proxy resolves names"
        )
    );
    assert_eq!(
        proxy_environment(
            false,
            env(&[("HTTP_PROXY", "http://p:1"), ("HTTPS_PROXY", "http://q:1")])
        )
        .warning()
        .as_deref(),
        Some(
            "web_fetch: HTTP_PROXY, HTTPS_PROXY is set; fetches through an HTTP(S) proxy are checked only on the URL's literal address; the proxy resolves names"
        )
    );
}
