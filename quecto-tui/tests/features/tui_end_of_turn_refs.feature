@tui @issue-1060 @adr-0008-part2 @done
Feature: TUI conversation rendering with ref-based end-of-turn events
  As a TUI operator watching a live agent turn
  I want the chat to look identical whether end-of-turn events carry full content or only message refs
  So that large turns stay small on the wire without changing what I see, and mid-turn attach still converges

  # Recovery design (mandatory, from abandoned PR #1075 — do NOT re-land
  # fetch-all + append-all):
  #   - Common streamed case: non-empty refs, zero message fetches, zero duplicates.
  #   - Mid-turn miss/partial: fetch only missing content; reconcile/replace at
  #     the turn's position — never blindly append.
  #   - Request-id gating on recovery responses.
  #   - Reconstruct all roles (assistant text, tool-call messages, tool results).
  #   - Prior-history attach/backfill (#1050) is out of scope here.

  # ─── Common streamed path (AC2) ────────────────────────────────────────────

  @done
  Scenario: A fully streamed text turn renders without fetching messages at end-of-turn
    Given a fresh TUI app harness connected for the active turn
    And the assistant has streamed tokens "Hello" then " world"
    When a turn_end arrives that identifies the assistant message by non-empty refs only
    Then the app master session shows "Hello world"
    And the app master session shows "Hello world" exactly once
    And the TUI issues no on-demand message fetches for the completed turn

  @done
  Scenario: A fully streamed tool turn renders tool calls and results without end-of-turn fetches
    Given a fresh TUI app harness connected for the active turn
    And the assistant has streamed a tool call for "bash" with result "ok"
    And the assistant has streamed tokens "done"
    When a turn_end arrives that identifies the tool and text messages by non-empty refs only
    Then the app master session shows the tool call "bash"
    And the app master session shows "done"
    And the TUI issues no on-demand message fetches for the completed turn

  @done
  Scenario: Footer context gauges still update from ref-based turn_end metadata
    Given a fresh TUI app harness connected for the active turn
    And the assistant has streamed tokens "ok"
    When a turn_end arrives with contextTokens 40000 and maxContextTokens 200000 and non-empty message refs
    Then the footer reflects context usage from the turn_end metadata

  # ─── Mid-turn connect / miss recovery (AC2) ────────────────────────────────

  @done
  Scenario: Connecting mid-turn converges to full content via refs and fetch-on-miss
    Given a TUI that connected mid-turn and missed early tokens of the active turn
    When a turn_end arrives that identifies the turn messages by non-empty refs
    Then the TUI requests only the missing message content for those refs
    When the matching recovery responses arrive for those requests
    Then the app master session shows the full assistant content for the active turn
    And the app master session shows that content exactly once

  @done
  Scenario: Mid-turn recovery reconstructs tool-call and tool-result messages in order
    Given a TUI that connected mid-turn and missed tool_execution events of the active turn
    When a turn_end arrives that identifies assistant tool-call and tool-result messages by non-empty refs
    Then the TUI requests only the missing message content for those refs
    When the matching recovery responses arrive for those requests
    Then the app master session shows the tool call and tool result in order
    And the app master session shows the final assistant text for the active turn exactly once

  @done
  Scenario: A recovery response for a different request id is ignored
    Given a TUI that connected mid-turn and missed early tokens of the active turn
    And a turn_end has arrived that identifies the turn messages by non-empty refs
    And the TUI has outstanding recovery requests for those refs
    When a get_message recovery response arrives with a non-matching request id
    Then the app master session does not apply that recovery payload
    And the TUI still awaits the recovery response for its own request id

  # ─── Sub-agent monitoring (AC3) ────────────────────────────────────────────

  @done
  Scenario: Sub-agent pane reflects child conversation from ref-based child turn_end without fetches
    Given a TUI viewing sub-agent "worker"
    And sub-agent "worker" has already streamed "partial" for the active child turn
    When a child turn_end arrives identifying those messages by refs only
    Then the sub-agent's session shows "partial"
    And the sub-agent's session shows "partial" exactly once
    And the TUI issues no on-demand message fetches for the completed child turn

  @done
  Scenario: Sub-agent pane recovers missing child content from child turn_end after mid-turn connect
    Given a TUI viewing sub-agent "worker" that connected mid-turn
    When a child turn_end arrives identifying the child messages by non-empty refs
    Then the TUI requests only the missing child message content for those refs
    When the matching recovery responses arrive for those requests
    Then the sub-agent's session shows the full child turn content
    And the sub-agent's session shows that content exactly once
