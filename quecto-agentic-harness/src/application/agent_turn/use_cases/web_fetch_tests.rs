use super::*;

fn target(scheme: &str, host: &str, candidate: Option<IpAddr>) -> WebDestinationTarget {
    WebDestinationTarget::new(scheme, host, 443, candidate)
}

fn authorization(scheme: &str, host: &str) -> WebDestinationAuthorization {
    WebDestinationPolicy::new().authorize(&target(scheme, host, None))
}

#[test]
fn owned_target_constructor_normalizes_and_exposes_adapter_facts() {
    let candidate = "1.1.1.1".parse().unwrap();
    let target = WebDestinationTarget::new("HTTPS", "EXAMPLE.COM", 443, Some(candidate));

    assert_eq!(target.scheme(), "https");
    assert_eq!(target.host(), "example.com");
    assert_eq!(target.effective_port(), Some(443));
    assert_eq!(target.candidate(), Some(candidate));
}

#[test]
fn scheme_allowlist_contains_exactly_http_and_https() {
    for scheme in ["http", "https", "HTTP", "Https"] {
        assert_eq!(
            authorization(scheme, "example.com"),
            WebDestinationAuthorization::Allowed,
            "expected normalized {scheme:?} to be allowed"
        );
    }
    for scheme in ["", "ftp", "file", "data", "javascript", "http ", "https+"] {
        assert_eq!(
            authorization(scheme, "example.com"),
            WebDestinationAuthorization::Denied,
            "expected {scheme:?} to be denied"
        );
    }
}

#[test]
fn public_literal_destinations_are_allowed() {
    for host in [
        "1.1.1.1",
        "8.8.8.8",
        "93.184.216.34",
        "[2606:4700:4700::1111]",
        "2606:4700:4700::1111",
    ] {
        assert_eq!(
            authorization("https", host),
            WebDestinationAuthorization::Allowed,
            "expected {host} to be public"
        );
    }
}

#[test]
fn every_non_public_ipv4_class_is_denied() {
    for host in [
        "0.0.0.0",         // unspecified / this network
        "0.1.2.3",         // this network
        "10.0.0.1",        // private
        "100.64.0.1",      // shared address space
        "127.0.0.1",       // loopback
        "127.255.255.254", // loopback range boundary
        "169.254.169.254", // link-local / metadata
        "172.16.0.1",      // private
        "172.31.255.255",  // private boundary
        "192.0.0.1",       // IETF protocol assignments
        "192.0.2.1",       // documentation
        "192.88.99.1",     // deprecated relay anycast
        "192.168.1.1",     // private
        "198.18.0.1",      // benchmarking
        "198.51.100.1",    // documentation
        "203.0.113.1",     // documentation
        "224.0.0.1",       // multicast
        "240.0.0.1",       // reserved
        "255.255.255.255", // broadcast
        "192.0.0.8",       // unclassified IETF protocol-assignment space
        "192.0.0.11",      // non-global IETF protocol-assignment space
    ] {
        assert_eq!(
            authorization("https", host),
            WebDestinationAuthorization::Denied,
            "expected {host} to be denied"
        );
    }
}

#[test]
fn every_non_public_ipv6_class_is_denied() {
    for host in [
        "::",                 // unspecified
        "::1",                // loopback
        "::ffff:127.0.0.1",   // IPv4-mapped loopback
        "64:ff9b::192.0.2.1", // translation prefix
        "100::1",             // discard-only
        "2001:db8::1",        // documentation
        "3fff::1",            // documentation
        "fc00::1",            // unique-local
        "fd12:3456::1",       // unique-local
        "fe80::1",            // link-local
        "ff02::1",            // multicast
        "2002:7f00:1::1",     // 6to4 encoding IPv4 loopback
        "2002:0a00:1::1",     // 6to4 encoding RFC-1918
        "2002:a9fe:a9fe::1",  // 6to4 encoding link-local metadata
        "2002:0808:0808::1",  // 6to4 fails closed even with public embedded IPv4
        "2000::1",            // unlisted 2000::/16
        "3000::1",            // IANA-reserved 3000::/5
        "3ffe::1",            // returned 6bone allocation
        "3fff::1",            // documentation
        "3fff:1000::1",       // unallocated space above 3fff::/20
    ] {
        assert_eq!(
            authorization("https", host),
            WebDestinationAuthorization::Denied,
            "expected {host} to be denied"
        );
    }
}

#[test]
fn ipv6_allocation_boundaries_are_affirmative_and_fail_closed_between_allocations() {
    let cases = [
        ("2001:1::1", true),
        ("2001:1::3", true),
        ("2001:1::4", false),
        ("2001:1ff:ffff:ffff:ffff:ffff:ffff:ffff", false),
        ("2001:200::", true),
        ("2001:db7:ffff:ffff:ffff:ffff:ffff:ffff", true),
        ("2001:db8::", false),
        ("2001:db8:ffff:ffff:ffff:ffff:ffff:ffff", false),
        ("2001:db9::", true),
        ("2001:bfff:ffff:ffff:ffff:ffff:ffff:ffff", true),
        ("2001:c000::", false),
        ("2002::", false),
        ("2002:ffff:ffff:ffff:ffff:ffff:ffff:ffff", false),
        ("2003::", true),
        ("2003:3fff:ffff:ffff:ffff:ffff:ffff:ffff", true),
        ("2003:4000::", false),
        ("23ff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", false),
        ("2400::", true),
        ("241f:ffff:ffff:ffff:ffff:ffff:ffff:ffff", true),
        ("2420::", false),
        ("2c0f:ffff:ffff:ffff:ffff:ffff:ffff:ffff", true),
        ("2c10::", false),
    ];

    for (host, expected_allowed) in cases {
        assert_eq!(
            authorization("https", host) == WebDestinationAuthorization::Allowed,
            expected_allowed,
            "unexpected authorization at IPv6 allocation boundary {host}"
        );
    }
}

#[test]
fn known_restricted_domain_names_are_denied_after_normalization() {
    for host in [
        "localhost",
        "LOCALHOST",
        "localhost.",
        "metadata.google.internal",
        "METADATA.GOOGLE.INTERNAL.",
    ] {
        assert_eq!(
            authorization("https", host),
            WebDestinationAuthorization::Denied,
            "expected {host:?} to be denied"
        );
    }
}

#[test]
fn syntactically_public_dns_names_are_allowed_pending_candidate_authorization() {
    for host in [
        "example.com",
        "api.example.com",
        "xn--bcher-kva.example",
        "example.com.",
    ] {
        assert_eq!(
            authorization("https", host),
            WebDestinationAuthorization::Allowed,
            "expected {host:?} to be provisionally allowed"
        );
    }
}

#[test]
fn malformed_or_unclassified_facts_fail_closed() {
    let policy = WebDestinationPolicy::new();
    let cases = [
        WebDestinationTarget::new("https", "", 443, None),
        WebDestinationTarget::new("https", "single-label", 443, None),
        WebDestinationTarget::new("https", ".example.com", 443, None),
        WebDestinationTarget::new("https", "example..com", 443, None),
        WebDestinationTarget::new("https", "-bad.example", 443, None),
        WebDestinationTarget::new("https", "bad-.example", 443, None),
        WebDestinationTarget::new("https", "bad_name.example", 443, None),
        WebDestinationTarget::new("https", "999.999.999.999", 443, None),
        WebDestinationTarget::new("https", "[not-ipv6]", 443, None),
        WebDestinationTarget::new("https", "example.com", None, None),
        WebDestinationTarget::new("https", "example.com", 0, None),
    ];

    for malformed in cases {
        assert_eq!(
            policy.authorize(&malformed),
            WebDestinationAuthorization::Denied,
            "expected malformed target {malformed:?} to fail closed"
        );
    }
}

#[test]
fn supplied_candidate_requires_its_own_affirmative_authorization() {
    let policy = WebDestinationPolicy::new();
    let public = target(
        "https",
        "public-looking.example",
        Some("1.1.1.1".parse().unwrap()),
    );
    let private = target(
        "https",
        "public-looking.example",
        Some("10.0.0.7".parse().unwrap()),
    );
    let loopback = target(
        "https",
        "public-looking.example",
        Some("::1".parse().unwrap()),
    );

    assert_eq!(
        policy.authorize(&public),
        WebDestinationAuthorization::Allowed
    );
    assert_eq!(
        policy.authorize(&private),
        WebDestinationAuthorization::Denied
    );
    assert_eq!(
        policy.authorize(&loopback),
        WebDestinationAuthorization::Denied
    );
}

#[test]
fn exact_test_destination_allows_only_that_host_and_port() {
    let policy = WebDestinationPolicy::with_test_destination(
        "allowed.test",
        8443,
        "127.0.0.1".parse().unwrap(),
    );
    let exact = WebDestinationTarget::new(
        "http",
        "ALLOWED.TEST",
        8443,
        Some("127.0.0.1".parse().unwrap()),
    );
    let wrong_host = WebDestinationTarget::new(
        "http",
        "other.test",
        8443,
        Some("127.0.0.1".parse().unwrap()),
    );
    let wrong_port = WebDestinationTarget::new(
        "http",
        "allowed.test",
        8444,
        Some("127.0.0.1".parse().unwrap()),
    );
    let wrong_candidate = WebDestinationTarget::new(
        "http",
        "allowed.test",
        8443,
        Some("127.0.0.2".parse().unwrap()),
    );
    let wrong_scheme = WebDestinationTarget::new(
        "ftp",
        "allowed.test",
        8443,
        Some("127.0.0.1".parse().unwrap()),
    );

    assert_eq!(
        policy.authorize(&exact),
        WebDestinationAuthorization::Allowed
    );
    assert_eq!(
        policy.authorize(&wrong_host),
        WebDestinationAuthorization::Denied
    );
    assert_eq!(
        policy.authorize(&wrong_port),
        WebDestinationAuthorization::Denied
    );
    assert_eq!(
        policy.authorize(&wrong_candidate),
        WebDestinationAuthorization::Denied
    );
    assert_eq!(
        policy.authorize(&wrong_scheme),
        WebDestinationAuthorization::Denied
    );
}

#[test]
fn malformed_test_permission_cannot_bypass_fail_closed_validation() {
    let empty_host =
        WebDestinationPolicy::with_test_destination("", 8443, "127.0.0.1".parse().unwrap());
    let zero_port =
        WebDestinationPolicy::with_test_destination("localhost", 0, "127.0.0.1".parse().unwrap());

    assert_eq!(
        empty_host.authorize(&WebDestinationTarget::new("http", "", 8443, None)),
        WebDestinationAuthorization::Denied
    );
    assert_eq!(
        zero_port.authorize(&WebDestinationTarget::new("http", "localhost", 0, None)),
        WebDestinationAuthorization::Denied
    );
}

#[test]
fn exact_literal_test_destination_may_reach_its_matching_loopback_server() {
    let policy = WebDestinationPolicy::with_test_destination(
        "127.0.0.1",
        32123,
        "127.0.0.1".parse().unwrap(),
    );
    let target = WebDestinationTarget::new(
        "http",
        "127.0.0.1",
        32123,
        Some("127.0.0.1".parse().unwrap()),
    );

    assert_eq!(
        policy.authorize(&target),
        WebDestinationAuthorization::Allowed
    );
}
