# Scenario: OpenAI OAuth path uses GPT-5.4 on the Responses API with cache-aware usage parsing

Given QuEcto is authenticated to OpenAI via OAuth
And the OAuth-backed OpenAI path uses the ChatGPT backend Responses API
When QuEcto builds a request for the OAuth-backed provider
Then it should target model `gpt-5.4`
And it should send a Responses API request body, not Chat Completions
And it should include a stable sanitized session-scoped cache key when a session id is present
And it should request non-persistent execution with `store: false`
And it should parse cached input token usage from the response into `cache_read_tokens`

Given the OAuth-backed provider replays prior assistant turns
When assistant history is sent to GPT-5.4 via the Responses API
Then commentary-style assistant turns should be marked with phase `commentary`
And final assistant answer turns should be marked with phase `final_answer`

Notes:
- OpenAI GPT-5.4 is documented for the Responses API and supports long-running/tool-heavy flows.
- Prompt caching is automatic; QuEcto should provide a stable cache key and surface cached token usage.
- QuEcto should continue avoiding server-side persistence by sending `store: false`.
