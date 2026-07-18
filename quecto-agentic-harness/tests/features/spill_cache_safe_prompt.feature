@done @context-pruning @cache-safe-prompt @issue-1118
Feature: Cache-safe spilled session memory
  As an operator of long agent sessions
  I want spilled session memory to remain discoverable without changing the cached prompt prefix
  So that growing history can reuse provider prompt caches

  Background:
    Given an isolated agent session with a controllable OpenAI provider
    And the model will complete after creating, discovering, and recalling spilled session memory with "memory handled"

  # AC1: every request sees the same static front guidance even though the
  # spill index grows between tool-loop turns.
  Scenario: Growing session memory leaves the provider-cached prefix unchanged
    When I run quecto agent -s spill-cache-safe --system "You are the cache-safe memory test agent." -m "exercise spilled session memory"
    Then every LLM request of the session should carry byte-identical front-positioned prompt guidance

  # AC2/AC4: the static tool contract is the discovery channel;
  # recall("list") supplies the live index and recall("<id>") retrieves content.
  Scenario: A model discovers and recalls spilled session memory on demand
    When I run quecto agent -s spill-cache-safe --system "You are the cache-safe memory test agent." -m "exercise spilled session memory"
    Then the recall tool should advertise its full session-memory index
    And the model should receive the complete live spill index on demand
    And the model should recall content using an id from that index
    And the live spill index should not appear in front-positioned prompt guidance

  # AC3 is regression-covered by the existing context_pruning.feature
  # collapse/demotion, spill-at-creation, persistence, and durable-prefix
  # scenarios; this slice deliberately leaves those behaviours unchanged.
