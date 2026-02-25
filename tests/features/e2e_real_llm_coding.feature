Feature: E2E Real LLM Coding Integration
  Comprehensive real-LLM checks for coding_job operational flows.

  Background:
    Given a real LLM workspace is configured
    And a git repo "test-repo" in the real LLM workspace with base ref "main"

  @wip @real-llm @real-llm-smoke
  Scenario: Real LLM can run and inspect a coding job in one request
    When I run the real LLM agent with message "Use the coding_job tool to run a job with goal 'real llm integration', repo 'test-repo', and base_ref 'main'. Then call status for that returned job_id. If status state is queued, reply exactly CODING_RUN_STATUS_OK, otherwise reply CODING_RUN_STATUS_FAIL."
    Then the exit code should be 0
    And stdout should contain "CODING_RUN_STATUS_"

  @wip @real-llm
  Scenario: Real LLM can cancel and cleanup a coding job in one request
    When I run the real LLM agent with message "Use coding_job to run a new job for repo test-repo base_ref main. Then cancel it, then cleanup it with keep_artifacts true. If both cancel and cleanup succeed reply exactly CODING_CANCEL_CLEANUP_OK, otherwise reply CODING_CANCEL_CLEANUP_FAIL."
    Then the exit code should be 0
    And stdout should contain "CODING_CANCEL_CLEANUP_"

  @wip @real-llm
  Scenario: Real LLM gets invalid_repo for unknown repository
    When I run the real LLM agent with message "Call coding_job run with goal 'x', repo 'missing-repo', base_ref 'main'. If tool returns invalid_repo, reply exactly CODING_INVALID_REPO_OK. Otherwise reply CODING_INVALID_REPO_FAIL."
    Then the exit code should be 0
    And stdout should contain "CODING_INVALID_REPO_"

  @wip @real-llm
  Scenario: Real LLM gets invalid_base_ref for bad branch
    When I run the real LLM agent with message "Call coding_job run with goal 'x', repo 'test-repo', base_ref 'does-not-exist'. If tool returns invalid_base_ref, reply exactly CODING_INVALID_REF_OK. Otherwise reply CODING_INVALID_REF_FAIL."
    Then the exit code should be 0
    And stdout should contain "CODING_INVALID_REF_"

  @wip @real-llm
  Scenario: Real LLM gets skill_not_found for missing coding skill
    When I run the real LLM agent with message "Call coding_job run with goal 'x', repo 'test-repo', base_ref 'main', and skills ['missing-skill']. If tool returns skill_not_found, reply exactly CODING_MISSING_SKILL_OK. Otherwise reply CODING_MISSING_SKILL_FAIL."
    Then the exit code should be 0
    And stdout should contain "CODING_MISSING_SKILL_"

  @wip @real-llm
  Scenario: Real LLM can persist coding context across named session turns
    When I run the real LLM agent with session codingmemo and message "Use coding_job run with repo test-repo base_ref main and goal 'session memory test'. Reply exactly SESSION_JOB_CREATED once done."
    Then the exit code should be 0
    And stdout should contain "SESSION_JOB_CREATED"
    When I run the real LLM agent with session codingmemo and message "Using the same session context, call coding_job status for the most recently created job. If state is queued reply exactly SESSION_STATUS_OK otherwise SESSION_STATUS_FAIL."
    Then the exit code should be 0
    And stdout should contain "SESSION_STATUS_"

  @wip @real-llm
  Scenario: Real LLM gateway can invoke coding_job from Telegram message flow
    Given a real LLM gateway workspace is configured for chat "1001" with message "Use coding_job to run a job for repo test-repo base_ref main and then report the state. Reply with exactly GATEWAY_CODING_OK if queued else GATEWAY_CODING_FAIL"
    And a git repo "test-repo" in the real LLM workspace with base ref "main"
    When I run quecto gateway until at least 1 Telegram replies are sent
    Then the Telegram outbound messages should include "GATEWAY_CODING_"
