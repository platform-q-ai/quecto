@done @workflow @workflow-nudge
Feature: Workflow auto-continue nudge wording
  As a workflow-bound agent driven by a literal instruction-following model
  I want the auto-continue nudge to demand action instead of a status reply
  So that a no-tool-call status answer never silently kills auto-continue mid-run

  # No feature-wide Background: scenarios below start from two mutually
  # exclusive workflow states (active vs awaiting template selection), so each
  # scenario declares its own Given.

  Scenario: Auto-continue nudge does not mandate a status-only reply
    Given an active workflow with incomplete steps and auto-continue enabled
    When I request the auto-continue nudge
    Then the nudge should not mandate a status-only reply

  Scenario: Auto-continue nudge instructs the model how to recover from a failed tool call
    Given an active workflow with incomplete steps and auto-continue enabled
    When I request the auto-continue nudge
    Then the nudge should instruct the model how to recover from a failed tool call

  Scenario: Corrective nudge demands a check-off or continued work instead of a status reply
    Given an active workflow with incomplete steps and auto-continue enabled
    When I request the corrective nudge
    Then the corrective nudge should demand a check-off or continued work

  # ─── Cache-safe prompting (#1113): nudges carry step state ──────────────────
  # The system prompt no longer carries workflow state, so idle-boundary nudges
  # (standard and corrective) must carry the current step (label + guidance),
  # and the template selector must reach an unselected session through the
  # first idle nudge instead of injected system-prompt text.

  @cache-safe-prompt
  Scenario: Auto-continue nudge carries the current step and its guidance
    Given an active workflow with incomplete steps and auto-continue enabled
    When I request the auto-continue nudge
    Then the nudge should carry the current step label and guidance

  @cache-safe-prompt
  Scenario: Corrective nudge carries the current step and its guidance
    Given an active workflow with incomplete steps and auto-continue enabled
    When I request the corrective nudge
    Then the nudge should carry the current step label and guidance

  @cache-safe-prompt
  Scenario: Auto-continue nudge presents the template selector before a template is selected
    Given a workflow awaiting template selection with auto-continue enabled
    When I request the auto-continue nudge
    Then the nudge should present the workflow template selector

  @cache-safe-prompt
  Scenario: Corrective nudge presents the template selector before a template is selected
    Given a workflow awaiting template selection with auto-continue enabled
    When I request the corrective nudge
    Then the nudge should present the workflow template selector
