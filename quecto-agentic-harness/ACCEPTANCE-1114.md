# Issue 1114 acceptance criteria

- `quecto models discover <provider-key>` discovers `/models` for providers whose `api` is `openai-completions` and whose `baseUrl` points at an OpenAI-compatible `/v1` endpoint.
- Discovery maps each returned model object to `{ "id": <opaque provider id>, "name": <display name> }`, where display name is `name` if present, else `owned_by` if present, else `id`; it does not generate `!command` auth or move secrets out of the existing registry.
- The merge replaces only the target provider's `models` array and preserves all other keys in that provider, all auth blocks, and every other provider entry.
- The rewritten `models.json` is serialized as valid JSON before publication and is published with a same-directory temp file plus rename.
- Users can repeat discovery with `--watch --interval <seconds>`, and docs include an automation recipe.
- Discovery remains outside the provider registry/reload kernel: no registry-loader network fetch, UDS provider registration, `!command`, or remote catalog cache.
