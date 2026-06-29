Feature: E2E Real LLM Entry Points
  Real endpoint checks that exercise CLI entry points directly.

  Background:
    Given a real LLM workspace is configured

  @done @manual-real-llm @mock-llm @real-llm-smoke
  Scenario: Real LLM works through agent subprocess entry point
    When I spawn quecto as a subprocess with args: agent --model gpt-5.2 --max-iterations 5 --max-time 60 -s - -m "Reply with exactly ENTRYPOINT_AGENT_OK"
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "ENTRYPOINT_AGENT_OK"

  @done @manual-real-llm @mock-llm @real-llm-smoke
  Scenario: Real LLM works through no-args REPL subprocess entry point
    When I spawn quecto as a subprocess with no args and stdin:
      """
      Reply with exactly ENTRYPOINT_REPL_OK
      /exit
      """
    Then the subprocess exit code should be 0
    And the subprocess stdout should contain "ENTRYPOINT_REPL_OK"
