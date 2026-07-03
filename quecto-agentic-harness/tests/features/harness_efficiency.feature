@done
Feature: Harness per-call efficiency cleanups (issue #996)
  Behaviour-preserving cleanups of repeated per-call waste and small
  duplications across the harness. Each scenario pins an observable invariant
  that must survive the refactor, stated in terms of what the harness does
  (bounded previews, token usage, config loading) rather than how it is coded.

  Scenario: A long multibyte value is previewed without splitting a character
    When a 500-character multibyte string is previewed to 100 characters
    Then the preview shows 100 characters ending in an ellipsis
    And every previewed character is a whole codepoint

  Scenario: OpenAI token usage is recorded from a provider usage report
    When an OpenAI response reports 12 prompt, 7 completion and 19 total tokens
    Then the recorded usage shows 12 prompt, 7 completion and 19 context tokens

  Scenario: Codex token usage is recorded from a provider usage report
    When a Codex response reports 100 input, 40 output and 30 cached tokens
    Then the recorded usage shows 100 prompt, 40 completion and 30 cached tokens

  Scenario: A config written by an older release still loads
    When a provider config written by an older release is loaded
    Then the config loads and its api_key and api_base are read back
