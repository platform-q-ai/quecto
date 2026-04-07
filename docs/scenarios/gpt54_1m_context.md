# Scenario: OpenAI GPT-5.4 becomes the default large-context model

Given OpenAI now documents `gpt-5.4` with a ~1.05M token context window
And QuEcto uses OpenAI as its default provider/model
When a user installs or upgrades QuEcto without overriding agent defaults
Then the default model should be `gpt-5.4`
And the default `max_context_tokens` budget should be raised to `1000000`
And the docs should describe `gpt-5.4` as the default OpenAI model
And the docs should describe the 1M-token application context budget as suitable for GPT-5.4 users

Notes:
- OpenAI docs indicate `gpt-5.4` / `gpt-5.4-pro` support a ~1.05M token context window.
- QuEcto still enforces an application-level budget; it does not auto-detect model context size.
- The OpenAI provider currently targets `/v1/chat/completions`, so this scenario only requires model/default-budget updates, not a provider API migration.
