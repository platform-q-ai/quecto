//! Headless device-code authentication flow.

use super::{CliContext, Output};

/// Device code login flow for headless environments.
pub(super) fn cmd_auth_login_device_code(ctx: &CliContext, provider: &str, out: &mut Output<'_>) -> i32 {
    let config = match resolve_oauth_config(ctx, provider, "device code flow", out.stderr) {
        Some(c) => c,
        None => return 1,
    };

    if config.device_code_url.is_empty() {
        out.stderr.push_str(&format!(
            "auth login: device code flow is not supported for '{}' (use --oauth instead)\n",
            provider
        ));
        return 1;
    }

    let rt = match super::build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: failed to create runtime: {}\n", e));
            return 1;
        }
    };
    match rt.block_on(crate::infrastructure::auth::oauth::request_device_code(
        &config,
    )) {
        Ok(resp) => {
            out.stdout.push_str(&format!(
                "Go to: {}\nEnter code: {}\n\nWaiting for authorization...\n",
                resp.verification_uri, resp.user_code
            ));
            0
        }
        Err(e) => {
            out.stderr
                .push_str(&format!("auth login: device code request failed: {}\n", e));
            1
        }
    }
}

