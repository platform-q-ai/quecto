**Title:** Document Anthropic OAuth token refresh mechanism

**Labels:** `documentation`, `auth`

---

## Summary

This issue documents the Anthropic OAuth access token refresh mechanism. Tokens are refreshed **lazily on demand** via the standard OAuth 2.0 refresh token grant — there is no background timer. When a token is needed and found to be expired, it is refreshed transparently before the API call proceeds.

## Refresh Flow

```
agent.rs ──► resolve_api_key_with_refresh() ──► is_expired()? ──► refresh_anthropic_token()
                        │                                                    │
                        │                                                    ▼
                        │                                          POST /v1/oauth/token
                        │                                                    │
                        ▼                                                    ▼
                  return access_token ◄──────── store new credential to disk
```

### Step by step

1. `resolve_api_key_with_refresh()` loads credentials from `~/.quecto/credentials.json`
2. If the token is **not expired** → return it immediately
3. If **expired**, verify preconditions:
   - Credential method is `AuthMethod::OAuth`
   - A `refresh_token` is present
   - Provider config exists for `"anthropic"`
4. `POST` to `https://console.anthropic.com/v1/oauth/token`:
   ```json
   {
     "grant_type": "refresh_token",
     "client_id": "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
     "refresh_token": "<stored refresh token>"
   }
   ```
5. **On success:** persist the new `access_token`, rotated `refresh_token`, and computed `expires_at` to disk, then return the new token
6. **On failure:** log a warning and fall back to `config_key` (e.g. `ANTHROPIC_API_KEY` env var)

## Design Decisions

- **Lazy refresh** — no background timer; appropriate for a CLI tool with no wasted network calls
- **5-minute early expiry buffer** — `expires_at = now + expires_in - 300` guards against clock skew and network latency
- **Refresh token rotation** — the new `refresh_token` replaces the old one on disk (Anthropic invalidates old refresh tokens after use)
- **Graceful fallback** — refresh failure degrades to `config_key` rather than hard-failing
- **Sync bridge** — async `reqwest` call is bridged via `rt.block_on()`
- **File permissions** — `credentials.json` is written with mode `0600`, re-enforced on every write
- **Secret redaction** — `Debug` impl on `Credential` redacts `token` and `refresh_token`

## Credential Storage

Stored in `~/.quecto/credentials.json`:

| Field | Description |
|---|---|
| `token` | Current access token (prefix `sk-ant-oat`) |
| `method` | `AuthMethod::OAuth` |
| `expires_at` | Unix timestamp (seconds), or `None` if no expiry |
| `refresh_token` | Used to obtain new access tokens |
| `account_id` | Always `None` for Anthropic |

## Initial Token Acquisition

The first token is obtained via `quecto auth login --provider anthropic` using a **browser-based PKCE authorization code flow**:

1. CLI generates PKCE verifier/challenge and a random state string
2. User authenticates at `https://claude.ai/oauth/authorize?...`
3. User pastes back a `code#state` string
4. CLI exchanges the code for `access_token`, `refresh_token`, and `expires_in`
5. Credentials are persisted; lazy refresh keeps the token alive from that point on

## Files Involved

| File | Role |
|---|---|
| `src/infrastructure/auth/credential_store.rs` | Credential persistence and `is_expired()` check |
| `src/infrastructure/auth/oauth.rs` | OAuth config, `refresh_anthropic_token()` |
| `src/interface/shared.rs` | `resolve_api_key_with_refresh()` orchestration |
| `src/interface/cli/agent.rs` | Call site (line 596) |
| `src/interface/cli/auth.rs` | Initial login flow |

## Test Coverage

**Unit tests** (`src/infrastructure/auth/oauth.rs`):
- `test_refresh_anthropic_token_success` — wiremock returns 200; verifies new tokens
- `test_refresh_anthropic_token_failure` — wiremock returns 400; verifies error handling

**Integration tests** (`tests/features/auth.feature`, `tests/bdd/auth_steps.rs`):
- End-to-end flows with mock OAuth servers

## Implementation Notes

- Anthropic refresh uses `application/json` (`.json()`), unlike OpenAI which uses `application/x-www-form-urlencoded` (`.form()`)
- HTTP error responses are truncated to 4096 bytes to prevent memory exhaustion
- Credentials are re-read from disk on every call (`store.load_snapshot()`) — no stale in-memory state
- Failure to persist refreshed credentials logs a warning but still returns the new token for the current session
