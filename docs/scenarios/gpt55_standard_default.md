# Scenario: GPT-5.5 standard is the default OpenAI model

Given OpenAI exposes the standard `gpt-5.5` model for general agentic use
And QuEcto uses OpenAI as its default provider/model
When a user starts QuEcto without overriding `agents.defaults.model`
Then the default model should be the standard `gpt-5.5` model
And README/workflow examples should document `gpt-5.5` rather than older GPT-5.2/GPT-5.4 defaults
And the package version should be bumped for the model/default documentation update

Notes:
- The default is the standard `gpt-5.5` model ID, not a pro or preview variant.
- Provider routing still accepts explicit `provider/model` overrides for users who need non-default models.
