// xAI (Grok) OAuth login flow — SuperGrok / X Premium+ subscription auth.

use super::super::CliContext;
use super::{
    OAuthStoreParams, Output, extract_fallback_code, flush_stdout, store_oauth_credential,
};

/// xAI OAuth login: PKCE + browser callback on 127.0.0.1:56121.
///
/// SuperGrok / X Premium+ subscription login against auth.x.ai. The resulting
/// bearer token authorizes requests to https://api.x.ai/v1.
pub(crate) fn cmd_auth_login_xai_oauth(
    ctx: &CliContext,
    config: &crate::infrastructure::auth::oauth::OAuthConfig,
    out: &mut Output<'_>,
) -> i32 {
    use crate::infrastructure::auth::oauth::{
        build_xai_auth_url, exchange_xai_code, generate_pkce, generate_state,
        parse_loopback_redirect, wait_for_oauth_callback_at,
    };

    let pkce = generate_pkce();
    let state = generate_state();
    let auth_url = build_xai_auth_url(config, &pkce, &state);

    out.stdout.push_str(&format!(
        "Open this URL in your browser to authenticate with xAI (SuperGrok / X Premium+):\n\n{}\n\n\
         Waiting for browser callback on {} ...\n\
         (If the browser doesn't open, copy the URL above and paste it manually)\n",
        auth_url, config.redirect_uri
    ));
    flush_stdout(ctx, out);

    let rt = match super::super::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: failed to create runtime: {}\n", e));
            return 1;
        }
    };

    // In test mode (stdin_data set), skip the browser callback and go
    // straight to the manual code-paste fallback.
    let code = if ctx.stdin_data.is_some() {
        let err = crate::domain::error::DomainError::Provider(
            "browser callback skipped in test mode".into(),
        );
        match extract_fallback_code(ctx, err, Some(&state), out) {
            Some(code) => code,
            None => return 1,
        }
    } else {
        // Derive the listener address/path from the registered redirect URI
        // so they cannot drift apart (PR #1087 review).
        let (addr, path) = match parse_loopback_redirect(&config.redirect_uri) {
            Ok(v) => v,
            Err(e) => {
                out.stderr
                    .push_str(&format!("auth login: invalid redirect URI: {}\n", e));
                return 1;
            }
        };
        match rt.block_on(wait_for_oauth_callback_at(&addr, &path, &state, 300)) {
            Ok(code) => code,
            Err(e) => match extract_fallback_code(ctx, e, Some(&state), out) {
                Some(code) => code,
                None => return 1,
            },
        }
    };

    match rt.block_on(exchange_xai_code(config, &code, &pkce.verifier)) {
        Ok(token_resp) => {
            let expires = crate::interface::shared::expires_at_with_margin(token_resp.expires_in);
            let params = OAuthStoreParams {
                provider: "xai".to_string(),
                account_id: None,
                expires_at: expires,
            };
            store_oauth_credential(ctx, params, &token_resp, out)
        }
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: token exchange failed: {}\n", e));
            1
        }
    }
}
