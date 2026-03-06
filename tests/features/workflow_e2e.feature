Feature: Workflow tool wired into agent
  As a user running the quecto agent
  I want the workflow tool available during agent runs
  So that I can track BDD/TDD progress interactively

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And workflow is enabled in config

  @done
  Scenario: Agent can call workflow status tool
    Given the mock LLM first returns a tool call for "workflow" with args:
      | action | status |
    And the mock LLM then returns a text response "Here is your workflow status"
    When I run quecto agent -s - -m "Show workflow status"
    Then the exit code should be 0
    And stdout should contain "workflow status"

  @done
  Scenario: Agent can check a workflow step
    Given the mock LLM first returns a tool call for "workflow" with args:
      | action | check |
      | step   | 1     |
    And the mock LLM then returns a text response "Step 1 is now checked"
    When I run quecto agent -s - -m "Check step 1"
    Then the exit code should be 0
    And stdout should contain "Step 1"

  @done
  Scenario: Agent can set workflow issue and check status
    Given the mock LLM returns a tool call sequence:
      | call | workflow | {"action":"set_issue","issueNumber":42,"issueTitle":"Add feature X"} |
      | call | workflow | {"action":"status"}                                                 |
      | text | Issue 42 is now tracked |                                                       |
    When I run quecto agent -s - -m "Track issue 42"
    Then the exit code should be 0
    And stdout should contain "Issue 42"

  @done
  Scenario: Workflow tool not registered when disabled in config
    Given workflow is disabled in config
    And the mock LLM returns a text response "I have no workflow tool"
    When I run quecto agent -s - -m "Show workflow status"
    Then the exit code should be 0

  @done
  Scenario: System prompt includes workflow progress
    Given a mock LLM that captures requests and returns text "I can see the workflow"
    When I run quecto agent -s - -m "What tools do you have?"
    Then the exit code should be 0
    And the LLM should have received a system message containing "Active Development Workflow"
