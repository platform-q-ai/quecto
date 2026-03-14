@done
Feature: E2E Real LLM Workflow Tool Coverage
  Comprehensive end-to-end tests that validate the workflow tool works
  correctly when driven by a real LLM. These scenarios exercise the full
  stack: LLM → tool call → WorkflowTool → WorkflowState → response.

  These scenarios are gated by @real-llm and only run when QUECTO_REAL_LLM=1.

  Background:
    Given a real LLM workspace is configured with workflow enabled

  # ── Basic actions ────────────────────────────────────────────────────────

  @done @real-llm
  Scenario: Real LLM checks workflow status and sees progress
    When I run the real LLM agent with message "Use the workflow tool with action 'status'. If the result contains step information and a progress indicator, reply with exactly WORKFLOW_STATUS_OK. Otherwise reply with WORKFLOW_STATUS_FAIL."
    Then the exit code should be 0
    And stdout should contain "WORKFLOW_STATUS_OK"

  @done @real-llm
  Scenario: Real LLM checks step 1 then verifies via status
    When I run the real LLM agent with message "Use the workflow tool: first call action 'check' with step 1. Then call action 'status'. If the status shows step 1 as done and progress 1/ reply with exactly WORKFLOW_PROGRESS_OK. Otherwise reply with WORKFLOW_PROGRESS_FAIL."
    Then the exit code should be 0
    And stdout should contain "WORKFLOW_PROGRESS_OK"

  @done @real-llm
  Scenario: Real LLM skips a step out of order
    When I run the real LLM agent with message "Use the workflow tool: call action 'skip' with step 5. Then call action 'status'. If step 5 shows as done in the status, reply with exactly WORKFLOW_SKIP_OK. Otherwise reply with WORKFLOW_SKIP_FAIL."
    Then the exit code should be 0
    And stdout should contain "WORKFLOW_SKIP_OK"

  @done @real-llm
  Scenario: Real LLM resets workflow after checking steps
    When I run the real LLM agent with message "Use the workflow tool: first call action 'check' with step 1. Then call action 'reset'. Then call action 'status'. If the status shows 0 steps done, reply with exactly WORKFLOW_RESET_OK. Otherwise reply with WORKFLOW_RESET_FAIL."
    Then the exit code should be 0
    And stdout should contain "WORKFLOW_RESET_OK"

  # ── Issue management ────────────────────────────────────────────────────

  @done @real-llm
  Scenario: Real LLM sets issue and verifies in status
    When I run the real LLM agent with message "Use the workflow tool: call action 'set_issue' with issueNumber 123 and issueTitle 'Fix login bug'. Then call action 'status'. If the status output contains '#123' and 'Fix login bug', reply with exactly WORKFLOW_SET_ISSUE_OK. Otherwise reply with WORKFLOW_SET_ISSUE_FAIL."
    Then the exit code should be 0
    And stdout should contain "WORKFLOW_SET_ISSUE_OK"

  # ── Error handling ──────────────────────────────────────────────────────

  @done @real-llm
  Scenario: Real LLM encounters ordering violation
    When I run the real LLM agent with message "Use the workflow tool: try to call action 'check' with step 3 directly without checking steps 1 and 2 first. The tool should return an error about ordering. If you get an error mentioning 'complete step 1 first', reply with exactly WORKFLOW_ORDER_OK. If it succeeds unexpectedly, reply with WORKFLOW_ORDER_FAIL."
    Then the exit code should be 0
    And stdout should contain "WORKFLOW_ORDER_OK"

  @done @real-llm
  Scenario: Real LLM encounters invalid step number
    When I run the real LLM agent with message "Use the workflow tool: try to call action 'check' with step 0 (zero). The tool should return an error about invalid step. If you get an error about invalid step, reply with exactly WORKFLOW_INVALID_OK. If it succeeds, reply with WORKFLOW_INVALID_FAIL."
    Then the exit code should be 0
    And stdout should contain "WORKFLOW_INVALID_OK"

  # ── Multi-step chaining ────────────────────────────────────────────────

  @done @real-llm
  Scenario: Real LLM chains multiple workflow operations
    When I run the real LLM agent with message "Use the workflow tool to do these steps in order: 1) set_issue with issueNumber 42 and issueTitle 'Feature X'. 2) check step 1. 3) check step 2. 4) status. If the status shows 2 steps done and mentions issue #42, reply with exactly WORKFLOW_CHAIN_OK. Otherwise reply with WORKFLOW_CHAIN_FAIL."
    Then the exit code should be 0
    And stdout should contain "WORKFLOW_CHAIN_OK"
