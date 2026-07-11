@done @workflow @workflow-nudge
Feature: Workflow auto-continue nudge wording
  As a workflow-bound agent driven by a literal instruction-following model
  I want the auto-continue nudge to demand action instead of a status reply
  So that a no-tool-call status answer never silently kills auto-continue mid-run

  Background:
    Given an active workflow with incomplete steps and auto-continue enabled

  Scenario: Auto-continue nudge does not mandate a status-only reply
    When I request the auto-continue nudge
    Then the nudge should not mandate a status-only reply

  Scenario: Auto-continue nudge instructs the model how to recover from a failed tool call
    When I request the auto-continue nudge
    Then the nudge should instruct the model how to recover from a failed tool call

  Scenario: Corrective nudge demands a check-off or continued work instead of a status reply
    When I request the corrective nudge
    Then the corrective nudge should demand a check-off or continued work
