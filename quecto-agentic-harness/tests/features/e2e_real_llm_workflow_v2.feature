@done
Feature: E2E Real LLM Workflow V2 UDS Tests
  Comprehensive end-to-end tests validating the Workflow V2 subsystem through
  a real LLM (OpenAI gpt-5.4 via OAuth). These exercise the full UDS stack:
  template selection, step progression, guards, live prompt injection,
  workflow_state events, get_state integration, and session persistence.

  Background:
    Given a real LLM UDS workspace is configured with workflow enabled

  # ═══════════════════════════════════════════════════════════════════════════
  # Template selection — selector mode
  # ═══════════════════════════════════════════════════════════════════════════

  @done @manual-real-llm @mock-llm
  Scenario: get_state shows selector mode before template selection
    When I start the real LLM UDS workflow agent
    And I send get_state with id "gs-sel"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the get_state response "gs-sel" should have workflow mode "selecting_template"
    And the get_state response "gs-sel" should have 10 available templates

  @done @manual-real-llm @mock-llm
  Scenario: LLM selects a workflow template
    When I start the real LLM UDS workflow agent
    And I send prompt "Call the workflow tool: action select_template, template feature, issueNumber 42, issueTitle 'Login timeout'. Reply TEMPLATE_SELECTED."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a workflow_state event with mode "active"
    And the agent output should contain a workflow_state event with template "feature"
    And the agent_end messages should contain "TEMPLATE_SELECTED"

  @done @manual-real-llm @mock-llm
  Scenario: LLM lists available templates
    When I start the real LLM UDS workflow agent
    And I send prompt "Call the workflow tool with action list_templates. If you see the feature and refactor templates, each with a description, reply TEMPLATE_LISTED. Otherwise reply TEMPLATE_FAIL."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent_end messages should contain "TEMPLATE_LISTED"

  # ═══════════════════════════════════════════════════════════════════════════
  # Step progression
  # ═══════════════════════════════════════════════════════════════════════════

  @done @manual-real-llm @mock-llm
  Scenario: LLM checks steps in order
    When I start the real LLM UDS workflow agent
    And I send prompt "Do the following in order using the workflow tool: 1) select_template feature 2) check step 1 3) check step 2 4) call status. If status shows 2 done, reply STEPS_CHECKED. Otherwise reply STEPS_FAIL."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent_end messages should contain "STEPS_CHECKED"

  @done @manual-real-llm @mock-llm
  Scenario: LLM gets ordering error when skipping ahead
    When I start the real LLM UDS workflow agent
    And I send prompt "Using the workflow tool: 1) select_template feature 2) try to check step 3 directly without checking step 1 first. If you get an ordering error mentioning 'complete step 1', reply ORDER_ERROR_OK. If it succeeds, reply ORDER_FAIL."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent_end messages should contain "ORDER_ERROR_OK"

  @done @manual-real-llm @mock-llm
  Scenario: LLM uses skip to bypass ordering
    When I start the real LLM UDS workflow agent
    And I send prompt "Using the workflow tool: 1) select_template feature 2) skip step 5 3) call status. If status shows step 5 as done, reply SKIP_OK. Otherwise reply SKIP_FAIL."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent_end messages should contain "SKIP_OK"

  @done @manual-real-llm @mock-llm
  Scenario: Step mutation emits workflow_state events with updated progress
    When I start the real LLM UDS workflow agent
    And I send prompt "Using the workflow tool: 1) select_template feature 2) check step 1 3) check step 2. Reply PROGRESS_DONE."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a workflow_state event with progress done 2

  # ═══════════════════════════════════════════════════════════════════════════
  # Issue tracking
  # ═══════════════════════════════════════════════════════════════════════════

  @done @manual-real-llm @mock-llm
  Scenario: LLM sets and clears an active issue
    When I start the real LLM UDS workflow agent
    And I send prompt "Using the workflow tool: 1) set_issue with issueNumber 99, issueTitle 'Fix auth'. 2) call status. If status shows #99, reply ISSUE_SET. Otherwise reply ISSUE_FAIL."
    And I send prompt "Now call the workflow tool with action clear_issue. Then call status. If the issue is gone, reply ISSUE_CLEARED. Otherwise reply CLEAR_FAIL."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent_end messages should contain "ISSUE_SET"
    And the agent_end messages should contain "ISSUE_CLEARED"

  # ═══════════════════════════════════════════════════════════════════════════
  # Reset
  # ═══════════════════════════════════════════════════════════════════════════

  @done @manual-real-llm @mock-llm
  Scenario: LLM resets workflow back to selector mode
    When I start the real LLM UDS workflow agent
    And I send prompt "Using the workflow tool: 1) select_template feature 2) check step 1 3) reset. Reply RESET_DONE."
    And I send get_state with id "gs-reset"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent output should contain a workflow_state event with mode "selecting_template"
    And the get_state response "gs-reset" should have workflow mode "selecting_template"

  # ═══════════════════════════════════════════════════════════════════════════
  # Completion
  # ═══════════════════════════════════════════════════════════════════════════

  @done @manual-real-llm @mock-llm
  Scenario: Completing all steps reaches complete mode
    When I start the real LLM UDS workflow agent
    And I send prompt "Using the workflow tool: 1) select_template feature 2) check every step from 1 through 17 in order. Then call status. If it says all steps complete, reply ALL_COMPLETE. Otherwise reply COMPLETE_FAIL."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent_end messages should contain "ALL_COMPLETE"
    And the agent output should contain a workflow_state event with mode "complete"

  # ═══════════════════════════════════════════════════════════════════════════
  # Guards
  # ═══════════════════════════════════════════════════════════════════════════

  @done @manual-real-llm @mock-llm
  Scenario: Guard blocks git commit before prerequisite steps
    When I start the real LLM UDS workflow agent
    And I send prompt "Using the workflow tool: 1) select_template feature 2) check_guards for command git commit. If you get a guard error about completing steps, reply GUARD_BLOCKED. If guards pass, reply GUARD_FAIL."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent_end messages should contain "GUARD_BLOCKED"

  @done @manual-real-llm @mock-llm
  Scenario: Guard passes after prerequisite steps are completed
    When I start the real LLM UDS workflow agent
    And I send prompt "Using the workflow tool: 1) select_template feature 2) check every step from 1 through 7 in order 3) check_guards for command git commit. If guards pass with 'satisfied', reply GUARD_PASS. Otherwise reply GUARD_STILL_BLOCKED."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent_end messages should contain "GUARD_PASS"

  # ═══════════════════════════════════════════════════════════════════════════
  # Live prompt injection
  # ═══════════════════════════════════════════════════════════════════════════

  @done @manual-real-llm @mock-llm
  Scenario: System prompt reflects workflow state changes between turns
    When I start the real LLM UDS workflow agent
    And I send prompt "Using the workflow tool: select_template feature with issueNumber 77 issueTitle 'Auth regression'. Reply SELECTED."
    And I send prompt "Without calling any tools, what is the current workflow template and issue number shown in your system prompt? Reply with exactly the template name and issue number."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent_end messages should contain "Feature"
    And the agent_end messages should contain "77"

  # ═══════════════════════════════════════════════════════════════════════════
  # get_state integration
  # ═══════════════════════════════════════════════════════════════════════════

  @done @manual-real-llm @mock-llm
  Scenario: get_state reflects template and progress after LLM actions
    When I start the real LLM UDS workflow agent
    And I send prompt "Using the workflow tool: 1) select_template feature with issueNumber 101 issueTitle 'New dashboard' 2) check step 1. Reply DONE."
    And I send get_state with id "gs-progress"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the get_state response "gs-progress" should have workflow mode "active"
    And the get_state response "gs-progress" should have workflow template "feature"
    And the get_state response "gs-progress" should have workflow progress done 1

  # ═══════════════════════════════════════════════════════════════════════════
  # Disabled workflow
  # ═══════════════════════════════════════════════════════════════════════════

  @done @manual-real-llm @mock-llm
  Scenario: UDS agent without --workflow has no workflow in get_state
    When I start the real LLM UDS agent
    And I send get_state with id "gs-nowf"
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the get_state response "gs-nowf" should not have workflow

  @done @manual-real-llm @mock-llm
  Scenario: UDS agent without --workflow has no workflow tool
    When I start the real LLM UDS agent
    And I send prompt "Try to call a tool called 'workflow' with action 'status'. If the tool does not exist or errors, reply NO_WORKFLOW_TOOL. If you get a status, reply HAS_WORKFLOW."
    And I close the UDS connection
    Then the UDS agent exits with code 0
    And the agent_end messages should contain "NO_WORKFLOW_TOOL"
