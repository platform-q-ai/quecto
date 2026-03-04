# Documentation: Anthropic OAuth Token Refresh Mechanism

## Summary

This issue documents how the Anthropic OAuth access token is refreshed when it expires. The implementation uses a **lazy, on-demand refresh** via the standard OAuth 2.0 refresh token grant. There is no background timer — tokens are refreshed only at the moment they are needed and found to be expired.

## Architecture

Four source files participate in the refresh flow:

| File | Role |
|---|---|
| `src/infrastructure/auth/credential_store.rs` | Persists credentials to disk; provides `is_expired()` check |
| `src/infrastructure/auth/oauth.rs` | OAuth configuration and HTTP calls (`refresh_anthropic_token()`) |
| `src/interface/shared.rs` | Orchestration (`resolve_api_key_with_refresh()`) — detects expiry, triggers refresh, persists result |
| `src/interface/cli/agent.rs` | Call site — invokes `resolve_api_key_with_refresh()` before building the Anthropic provider |

## Detailed Flow

### 1. Credential Storage

Credentials are stored in `<base_dir>/credentials.json` (default: `~/.quecto/credentials.json`), written with Unix file mode `0600`. Each credential record contains:

- **`token`** — the current access token (Anthropic OAuth tokens have the prefix `sk-ant-oat`)
- **`method`** — `AuthMethod::OAuth` for OAuth-obtained credentials
- **`expires_at`** — Unix timestamp (seconds) when the token expires, or `None` if no expiry
- **`refresh_token`** — the refresh token used to obtain new access tokens
- **`account_id`** — always `None` for Anthropic (used only for OpenAI)

Expiration is checked by `Credential::is_expired()`, which compares `expires_at` against `chrono::Utc::now().timestamp()`. If `expires_at` is `None`, the token is considered never-expired.

### 2. OAuth Configuration

The Anthropic OAuth config is hardcoded in `OAuthConfig::for_provider("anthropic")`:

| Field | Value |
|---|---|
| `authorization_url` | `https://claude.ai/oauth/authorize` |
| `token_url` | `https://console.anthropic.com/v1/oauth/token` |
| `client_id` | `9d1c250a-e61b-44d9-88ed-5944d1962f5e` |
| `redirect_uri` | `https://console.anthropic.com/oauth/code/callback` |
| `scopes` | `org:create_api_key user:profile user:inference` |
| `device_code_url` | `""` (empty — Anthropic does not support device code flow) |

### 3. The Refresh HTTP Call

`refresh_anthropic_token()` in `src/infrastructure/auth/oauth.rs` (line 167) performs the actual token refresh:

- **Method:** `POST`
- **URL:** `https://console.anthropic.com/v1/oauth/token`
- **Content-Type:** `application/json` (set both explicitly via `.header()` and implicitly via `.json()`)
- **Timeout:** 30 seconds
- **Request body:**
  ```json
  {
      "grant_type": "refresh_token",
      "client_id": "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
      "refresh_token": "<stored refresh token>"
  }
  ```

> **Note:** This differs from OpenAI's refresh, which uses `application/x-www-form-urlencoded` via `.form()`.

- **On HTTP 200**, the response is deserialized into `OAuthTokenResponse`:
  ```rust
  pub struct OAuthTokenResponse {
      pub access_token: String,   // new access token
      pub refresh_token: String,  // new (rotated) refresh token
      pub expires_in: u64,        // seconds until expiry
  }
  ```
- **On failure**, returns a `DomainError::Provider` containing the HTTP status code and a truncated error body (max 4096 bytes, to prevent memory exhaustion from malicious servers).

### 4. Orchestration — `resolve_api_key_with_refresh()`

This function in `src/interface/shared.rs` (line 103) is called from `src/interface/cli/agent.rs` (line 596) before building the Anthropic provider:

```rust
let anthropic_key = resolve_api_key_with_refresh(
    &config.providers.anthropic.api_key,  // config_key fallback
    &store,                                // CredentialStore
    "anthropic",                           // provider name
    &rt,                                   // tokio Runtime
);
```

**Step-by-step flow:**

1. **Load credentials** from disk via `store.load_snapshot()` (re-reads file each time — no stale state)
2. **Check expiration** — if `!cred.is_expired()`, return the stored token immediately (happy path)
3. **If expired**, verify three preconditions:
   - `cred.method == AuthMethod::OAuth` (only OAuth tokens are refreshable)
   - `cred.refresh_token` is `Some(...)` (a refresh token exists)
   - `OAuthConfig::for_provider("anthropic")` returns `Some(config)` (provider is recognized)
4. **Dispatch refresh** — for Anthropic, the `_ =>` (default) match arm calls `refresh_anthropic_token()`. The async call is bridged to sync via `rt.block_on()`.
5. **On success:**
   - Compute `expires_at = chrono::Utc::now().timestamp() + expires_in - 300` (5-minute safety buffer — token is considered expired 5 minutes before the server would actually reject it)
   - Set `account_id = None` for Anthropic
   - Build a new `Credential` with the fresh `access_token`, the **new rotated `refresh_token`**, and the new `expires_at`
   - Persist to disk via `store.store(new_cred)` (failure to persist logs a warning but still returns the new token)
   - Return the new `access_token`
6. **On failure:**
   - Log `tracing::warn!("failed to refresh OAuth token for anthropic: {}", e)`
   - Fall back to `config_key` (typically the `ANTHROPIC_API_KEY` environment variable or config file value)

## Design Decisions

| Decision | Rationale |
|---|---|
| **Lazy refresh (no background timer)** | Appropriate for a CLI tool — tokens are refreshed only when actually needed. No wasted network calls. |
| **5-minute early expiry buffer** | `expires_at` is set to `now + expires_in - 300`. This prevents edge-case failures from clock skew, network latency, or tokens expiring mid-request. |
| **Refresh token rotation** | The new `refresh_token` from the response replaces the old one on disk. This is required by Anthropic's OAuth implementation — old refresh tokens become invalid after use. |
| **Graceful fallback** | If refresh fails (network error, `invalid_grant`, etc.), the system falls back to `config_key` rather than hard-failing. This allows users with both OAuth and env-var credentials to degrade gracefully. |
| **Synchronous bridge** | The refresh call is async (`reqwest`) but `resolve_api_key_with_refresh()` is synchronous, so it uses `rt.block_on()` to bridge. The `tokio::runtime::Runtime` is created in `agent.rs` and passed in. |
| **File permissions** | `credentials.json` is created with Unix mode `0600` and permissions are re-enforced on every write to guard against external weakening. |
| **Secret redaction** | The `Debug` impl on `Credential` redacts both `token` and `refresh_token` to prevent accidental exposure in logs, panic backtraces, or `unwrap()` failure messages. |

## Initial Token Acquisition (for context)

The initial Anthropic OAuth token is obtained via a **browser-based PKCE authorization code flow** (`quecto auth login --provider anthropic`), implemented in `src/interface/cli/auth.rs`:

1. CLI generates PKCE codes (`generate_pkce()` — 32 random bytes → base64url verifier → SHA-256 → base64url S256 challenge) and a random hex state string
2. User is directed to `https://claude.ai/oauth/authorize?...` with the PKCE challenge, `response_type=code`, `redirect_uri`, `scope`, and `state`
3. User authenticates in the browser and pastes back a `code#state` string
4. CLI exchanges the code via `exchange_anthropic_code()` — POST JSON to the token URL with `grant_type=authorization_code`, `code`, `state`, `redirect_uri`, and `code_verifier`
5. Response returns `access_token`, `refresh_token`, and `expires_in`
6. Stored to `credentials.json` with `method: OAuth`, computed `expires_at` (with the same 5-minute buffer), and the `refresh_token`

From that point forward, the lazy refresh mechanism described above keeps the token alive automatically.

## Test Coverage

The refresh path has unit tests in `src/infrastructure/auth/oauth.rs`:

- `test_refresh_anthropic_token_success` — wiremock server returns HTTP 200 with new tokens; verifies new `access_token` and `refresh_token` are returned
- `test_refresh_anthropic_token_failure` — wiremock server returns HTTP 400; verifies error is returned containing the status code

Integration-level coverage exists in `tests/features/auth.feature` and `tests/bdd/auth_steps.rs` with mock OAuth servers.

## Related Files

- `src/infrastructure/auth/oauth.rs` — OAuth config, PKCE, token exchange, refresh functions
- `src/infrastructure/auth/credential_store.rs` — Credential struct, file-based persistence, expiration logic
- `src/interface/shared.rs` — `resolve_api_key_with_refresh()` orchestration
- `src/interface/cli/agent.rs` — Call site (line 596)
- `src/interface/cli/auth.rs` — Initial login flow, `store_oauth_credential()`
- `docs/SECURITY.md` — Documents credential file location and permissions
