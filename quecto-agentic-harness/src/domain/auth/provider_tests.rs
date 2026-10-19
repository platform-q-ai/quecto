use super::*;

#[test]
fn every_login_provider_parses_from_its_own_id() {
    for provider in LoginProvider::ALL {
        assert_eq!(LoginProvider::parse(provider.id()), Some(provider));
    }
}

#[test]
fn the_login_providers_are_openai_anthropic_and_xai_in_that_order() {
    let ids: Vec<_> = LoginProvider::ALL.iter().map(|p| p.id()).collect();
    assert_eq!(ids, ["openai", "anthropic", "xai"]);
}

#[test]
fn a_name_outside_the_allowlist_is_no_login_provider() {
    for name in [
        "",
        "bogus",
        "grok",
        "OpenAI",
        " openai",
        "openai ",
        "openai-oauth",
    ] {
        assert_eq!(LoginProvider::parse(name), None, "{name:?}");
    }
}

#[test]
fn only_openai_offers_a_device_code_login() {
    let offered: Vec<_> = LoginProvider::ALL
        .into_iter()
        .filter(|provider| provider.offers_device_code())
        .collect();
    assert_eq!(offered, [LoginProvider::OpenAi]);
}
