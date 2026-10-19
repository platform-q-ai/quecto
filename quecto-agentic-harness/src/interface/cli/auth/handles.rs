//! The auth handles `quecto auth` holds (#1839): declared here as plain
//! structs of use-case handles, filled by composition
//! (`composition::bootstrap::auth`) through the builder `main` hands to
//! the CLI. The interface never constructs a use case.

use std::sync::Arc;

use crate::application::auth::use_cases::{
    InspectAuthStatus, LogInWithOAuth, LogInWithToken, StartDeviceCodeLogin,
};
use crate::interface::cli::CliContext;

/// Composition's builder of the auth handles, injected through the CLI
/// context; the interface never opens the credential store itself.
pub type AuthHandlesBuilder = fn(&AuthEnvironment) -> AuthHandles;

/// What composition builds a run's auth handles over (#1839): its base
/// directory, whose `credentials.json` holds the credentials, and the one
/// OAuth server a test points every provider at.
#[derive(Debug, Clone)]
pub struct AuthEnvironment {
    pub base_dir: std::path::PathBuf,
    pub oauth_base_url: Option<String>,
}

/// The auth use cases `quecto auth` invokes (#1839); composition
/// (`composition::bootstrap::auth`) fills it through the builder `main`
/// hands to the CLI. The interface never constructs one.
#[derive(Debug, Clone)]
pub struct AuthHandles {
    /// `auth login --provider P --token T`.
    pub token_login: Arc<LogInWithToken>,
    /// `auth login --provider P --device-code`.
    pub device_code_login: Arc<StartDeviceCodeLogin>,
    /// `auth login [--provider P]`: the browser OAuth login.
    pub oauth_login: Arc<LogInWithOAuth>,
    /// `auth status`.
    pub status: Arc<InspectAuthStatus>,
}

/// The composed auth handles for `ctx`'s base directory; without them,
/// `auth login`'s refusal is written to `stderr` and there are none.
pub(super) fn login_handles(ctx: &CliContext, stderr: &mut String) -> Option<AuthHandles> {
    composed(ctx, "auth login", stderr)
}

/// [`login_handles`] for `auth status`.
pub(super) fn status_handles(ctx: &CliContext, stderr: &mut String) -> Option<AuthHandles> {
    composed(ctx, "auth status", stderr)
}

fn composed(ctx: &CliContext, command: &str, stderr: &mut String) -> Option<AuthHandles> {
    let Some(build) = ctx.auth else {
        stderr.push_str(&format!("{command}: auth capability not composed\n"));
        return None;
    };
    Some(build(&AuthEnvironment {
        base_dir: ctx.base_dir(),
        oauth_base_url: ctx.oauth_base_url.clone(),
    }))
}
