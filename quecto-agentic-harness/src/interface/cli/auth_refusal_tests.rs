//! `auth login`'s refusals, word for word (#1839 review).

use super::*;
use crate::domain::auth::pasted_code::PasteRefusal;

/// Every refusal a login can give, exactly as `auth login` prints it.
#[test]
fn each_login_refusal_keeps_its_wording() {
    let cases = [
        (
            LoginError::UnknownProvider("groq".to_string()),
            "auth login: unknown provider 'groq'. Known: openai, anthropic, xai\n",
        ),
        (
            LoginError::EmptyToken,
            "auth login: --token value must not be empty\n",
        ),
        (
            LoginError::DeviceCodeUnsupported(LoginProvider::Xai),
            "auth login: device code flow is not supported for 'xai' (use --oauth instead)\n",
        ),
        (
            LoginError::Unavailable("EAGAIN".to_string()),
            "auth login: failed to create runtime: EAGAIN\n",
        ),
        (
            LoginError::InsideAsyncRuntime,
            "auth login: cannot start a login from inside an async runtime\n",
        ),
        (
            LoginError::DeviceCodeRequest(
                "provider error: device code request failed (500)".to_string(),
            ),
            "auth login: device code request failed: provider error: device code request failed (500)\n",
        ),
        (
            LoginError::OAuthUnsupported(LoginProvider::Anthropic),
            "auth login: OAuth is not supported for 'anthropic'\n",
        ),
        (
            LoginError::Paste(PasteRefusal::StateMismatch),
            "auth login: state mismatch in pasted redirect URL\n",
        ),
        (
            LoginError::Paste(PasteRefusal::StateMissing),
            "auth login: pasted redirect URL is missing the state parameter\n",
        ),
        (
            LoginError::Paste(PasteRefusal::NoCode),
            "auth login: could not extract authorization code\n",
        ),
        (
            LoginError::EmptyAuthorizationCode,
            "auth login: authorization code must not be empty\n",
        ),
        (
            LoginError::InvalidRedirect(
                "provider error: redirect URI host is not loopback".to_string(),
            ),
            "auth login: invalid redirect URI: provider error: redirect URI host is not loopback\n",
        ),
        (
            LoginError::TokenExchange("provider error: token exchange failed (400)".to_string()),
            "auth login: token exchange failed: provider error: token exchange failed (400)\n",
        ),
        (
            LoginError::Store("config error: failed to parse credentials".to_string()),
            "auth login: failed to store credential: config error: failed to parse credentials\n",
        ),
    ];
    for (error, printed) in cases {
        assert_eq!(login_refusal(&error), printed, "{error:?}");
    }
}
