use super::*;

#[test]
fn test_create_openai_provider() {
    let provider = create_provider_with_client(
        "openai",
        "sk-test".to_string(),
        None,
        reqwest::Client::new(),
    );
    assert!(provider.is_ok());
    assert_eq!(provider.unwrap().name(), "openai");
}

#[test]
fn test_create_anthropic_provider() {
    let provider = create_provider_with_client(
        "anthropic",
        "sk-ant-test".to_string(),
        None,
        reqwest::Client::new(),
    );
    assert!(provider.is_ok());
    assert_eq!(provider.unwrap().name(), "anthropic");
}

#[test]
fn invalid_api_base_display_redacts_url_credentials_query_and_fragment() {
    let err = ProviderFactoryError::InvalidApiBase {
        provider: "openai".to_string(),
        api_base: "https://user:secret@example.test/v1?api_key=top#frag".to_string(),
        reason: "boom".to_string(),
    }
    .to_string();
    assert!(err.contains("https://example.test/v1"), "{err}");
    assert!(!err.contains("user"), "{err}");
    assert!(!err.contains("secret"), "{err}");
    assert!(!err.contains("api_key"), "{err}");
    assert!(!err.contains("frag"), "{err}");
}

#[test]
fn test_create_unknown_provider() {
    let provider =
        create_provider_with_client("gemini", "key".to_string(), None, reqwest::Client::new());
    assert!(matches!(
        provider,
        Err(ProviderFactoryError::UnknownProvider(_))
    ));
}

#[test]
fn test_create_openai_with_custom_base() {
    let provider = create_provider_with_client(
        "openai",
        "sk-test".to_string(),
        Some("http://localhost:8080".to_string()),
        reqwest::Client::new(),
    );
    assert!(provider.is_ok());
}

#[test]
fn test_create_openai_compatible_provider_with_custom_prefix() {
    let provider = create_openai_compatible_provider(
        "spark",
        "sk-spark".to_string(),
        "http://127.0.0.1:8000/v1".to_string(),
        false,
        reqwest::Client::new(),
    );
    assert!(provider.is_ok());
    assert_eq!(provider.unwrap().name(), "spark");
}

#[test]
fn test_create_openai_compatible_rejects_reserved_prefix() {
    let provider = create_openai_compatible_provider(
        "openai",
        "sk-spark".to_string(),
        "http://127.0.0.1:8000/v1".to_string(),
        false,
        reqwest::Client::new(),
    );
    assert!(matches!(
        provider,
        Err(ProviderFactoryError::UnknownProvider(_))
    ));
}

#[test]
fn test_create_openai_compatible_remote_http_requires_opt_in() {
    let rejected = create_openai_compatible_provider(
        "spark",
        "sk-spark".to_string(),
        "http://tailnet-host:8000/v1".to_string(),
        false,
        reqwest::Client::new(),
    );
    assert!(matches!(
        rejected,
        Err(ProviderFactoryError::InvalidApiBase { .. })
    ));

    let allowed = create_openai_compatible_provider(
        "spark",
        "sk-spark".to_string(),
        "http://tailnet-host:8000/v1".to_string(),
        true,
        reqwest::Client::new(),
    );
    assert!(allowed.is_ok());
}

#[test]
fn test_reject_openai_with_insecure_http_api_base() {
    let provider = create_provider_with_client(
        "openai",
        "sk-test".to_string(),
        Some("http://attacker.invalid/v1".to_string()),
        reqwest::Client::new(),
    );
    assert!(matches!(
        provider,
        Err(ProviderFactoryError::InvalidApiBase { .. })
    ));
}

#[test]
fn test_reject_anthropic_with_insecure_http_api_base() {
    let provider = create_provider_with_client(
        "anthropic",
        "sk-ant-test".to_string(),
        Some("http://attacker.invalid".to_string()),
        reqwest::Client::new(),
    );
    assert!(matches!(
        provider,
        Err(ProviderFactoryError::InvalidApiBase { .. })
    ));
}

#[test]
fn test_reject_openai_with_unapproved_https_host() {
    let provider = create_provider_with_client(
        "openai",
        "sk-test".to_string(),
        Some("https://evil.example/v1".to_string()),
        reqwest::Client::new(),
    );
    assert!(matches!(
        provider,
        Err(ProviderFactoryError::InvalidApiBase { .. })
    ));
}
