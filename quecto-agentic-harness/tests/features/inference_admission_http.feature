@inference-admission-http @done
Feature: HTTP feedback belongs to each admitted physical provider attempt

  Scenario Outline: Provider HTTP outcomes preserve admission ownership
    Given an admitted <provider> <surface> HTTP call
    When the loopback server returns <reply>
    Then HTTP admission records exactly one owned outcome without replay

    Examples:
      | provider | surface | reply |
      | OpenAI | chat | success |
      | OpenAI | chat | opaque-throttle |
      | OpenAI | chat | terminal-billing |
      | OpenAI | chat | header-throttle |
      | OpenAI | assembled | success |
      | OpenAI | assembled | opaque-throttle |
      | OpenAI | assembled | terminal-billing |
      | OpenAI | assembled | header-throttle |
      | OpenAI | incremental | success |
      | OpenAI | incremental | opaque-throttle |
      | OpenAI | incremental | terminal-billing |
      | OpenAI | incremental | header-throttle |
      | Anthropic | chat | success |
      | Anthropic | chat | opaque-throttle |
      | Anthropic | chat | terminal-billing |
      | Anthropic | chat | header-throttle |
      | Anthropic | assembled | success |
      | Anthropic | assembled | opaque-throttle |
      | Anthropic | assembled | terminal-billing |
      | Anthropic | assembled | header-throttle |
      | Anthropic | incremental | success |
      | Anthropic | incremental | opaque-throttle |
      | Anthropic | incremental | terminal-billing |
      | Anthropic | incremental | header-throttle |
      | Codex-key | chat | success |
      | Codex-key | chat | opaque-throttle |
      | Codex-key | chat | terminal-billing |
      | Codex-key | chat | header-throttle |
      | Codex-key | assembled | success |
      | Codex-key | assembled | opaque-throttle |
      | Codex-key | assembled | terminal-billing |
      | Codex-key | assembled | header-throttle |
      | Codex-key | incremental | success |
      | Codex-key | incremental | opaque-throttle |
      | Codex-key | incremental | terminal-billing |
      | Codex-key | incremental | header-throttle |
      | Codex-oauth | chat | success |
      | Codex-oauth | chat | opaque-throttle |
      | Codex-oauth | chat | terminal-billing |
      | Codex-oauth | chat | header-throttle |
      | Codex-oauth | assembled | success |
      | Codex-oauth | assembled | opaque-throttle |
      | Codex-oauth | assembled | terminal-billing |
      | Codex-oauth | assembled | header-throttle |
      | Codex-oauth | incremental | success |
      | Codex-oauth | incremental | opaque-throttle |
      | Codex-oauth | incremental | terminal-billing |
      | Codex-oauth | incremental | header-throttle |
