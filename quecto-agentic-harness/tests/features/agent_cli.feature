@done
Feature: Agent CLI — Headless One-Shot Mode
  As a user or automated system
  I want to run a full agent cycle from the command line with a single message
  So that I can use Quecto non-interactively for scripting, testing, and subagent spawning

  Scenario: One-shot message returns LLM response on stdout
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "The answer is 42"
    When I run quecto agent -m "What is the answer?"
    Then the exit code should be 0
    And stdout should contain "The answer is 42"

  Scenario: Missing message flag shows usage error
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I run quecto agent with no flags
    Then the exit code should be 1
    And stderr should contain "agent: -m is required for non-interactive mode"

  Scenario: Missing config runs on defaults (zero-config), needs a provider key
    Given a temp base directory
    When I run quecto agent -m "hello"
    Then the exit code should be 1
    And stderr should contain "no LLM providers"

  Scenario: No configured providers shows clear error
    Given a temp base directory
    And a config file with no API keys
    When I run quecto agent -m "hello"
    Then the exit code should be 1
    And stderr should contain "no LLM providers"

  Scenario: System prompt is prepended to conversation
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "I am a pirate assistant"
    When I run quecto agent --system "You are a pirate" -m "Who are you?"
    Then the exit code should be 0
    And stdout should contain "I am a pirate assistant"

  Scenario: Model override via flag
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "Hello from override model"
    When I run quecto agent --model gpt-5-mini -m "Hi"
    Then the exit code should be 0
    And stdout should contain "Hello from override model"

  Scenario: QUECTO_BASE_DIR environment variable overrides default base directory
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "env override works"
    When I set QUECTO_BASE_DIR to the temp directory
    And I run quecto agent -m "test"
    Then the exit code should be 0
    And stdout should contain "env override works"

  Scenario: Provider error returns non-zero exit code
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns an HTTP 500 error
    When I run quecto agent -m "hello"
    Then the exit code should be 1
    And stderr should contain "Error"

  # --- Issue #191: --no-session flag ---

  Scenario: --no-session flag is accepted and runs ephemerally
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ephemeral reply"
    When I run quecto agent --no-session -m "hello"
    Then the exit code should be 0
    And stdout should contain "ephemeral reply"

  Scenario: --no-session and -s are mutually exclusive
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I run quecto agent --no-session -s mysession -m "hello"
    Then the exit code should be 1
    And stderr should contain "--no-session and -s are mutually exclusive"

  Scenario: --no-session flag is documented in help
    Given a temp base directory
    When I run quecto help
    Then the exit code should be 0
    And stdout should contain "--no-session"

  # --- Issue #0: --no-sandbox flag ---

  Scenario: --no-sandbox flag is accepted and disables workspace restriction
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "sandbox disabled"
    When I run quecto agent --no-sandbox -m "hello"
    Then the exit code should be 0
    And stdout should contain "sandbox disabled"

  Scenario: --no-sandbox flag parses correctly alongside other flags
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ok"
    When I run quecto agent --no-sandbox --no-session -m "hello"
    Then the exit code should be 0
    And stdout should contain "ok"

  Scenario: --no-sandbox flag is documented in help
    Given a temp base directory
    When I run quecto help
    Then the exit code should be 0
    And stdout should contain "--no-sandbox"

  # --- Issue #300: --config flag ---

  Scenario: --config flag loads config from custom path
    Given a temp base directory
    And a config file at a custom path with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "custom config works"
    When I run quecto agent --config <custom-config-path> -m "hello"
    Then the exit code should be 0
    And stdout should contain "custom config works"

  Scenario: --config flag with nonexistent path shows error
    Given a temp base directory
    When I run quecto agent --config /tmp/nonexistent-config.json -m "hello"
    Then the exit code should be 1
    And stderr should contain "config not found"

  Scenario: --config flag is documented in help
    Given a temp base directory
    When I run quecto help
    Then the exit code should be 0
    And stdout should contain "--config"

  Scenario: --config with --model override (flag takes priority)
    Given a temp base directory
    And a config file at a custom path with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "model override works"
    When I run quecto agent --config <custom-config-path> --model gpt-5-mini -m "Hi"
    Then the exit code should be 0
    And stdout should contain "model override works"

  # --- Issue #402: --disable-tool flag ---

  Scenario: --disable-tool hides a tool from the agent model
    Given a temp base directory
    And a mock LLM that captures requests and returns text "no bash for you"
    When I run quecto agent --disable-tool bash -m "hello"
    Then the exit code should be 0
    And stdout should contain "no bash for you"
    And the LLM request should not have included tool "bash"

  Scenario: --disable-tool warns on unknown tool name
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "ok"
    When I run quecto agent --disable-tool nonexistent_tool -m "hello"
    Then the exit code should be 0
    And stderr should contain "no tool named 'nonexistent_tool'"

  Scenario: --disable-tool is repeatable
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "restricted"
    When I run quecto agent --disable-tool bash --disable-tool web_fetch -m "hello"
    Then the exit code should be 0
    And stdout should contain "restricted"

  Scenario: --disable-tool is documented in help
    Given a temp base directory
    When I run quecto help
    Then the exit code should be 0
    And stdout should contain "--disable-tool"

  # --- Issue #416: --effort flag ---

  Scenario: --effort flag is accepted and passes through
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "effort accepted"
    When I run quecto agent --effort medium -m "hello"
    Then the exit code should be 0
    And stdout should contain "effort accepted"

  Scenario: --effort flag with invalid value shows error
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I run quecto agent --effort turbo -m "hello"
    Then the exit code should be 1
    And stderr should contain "invalid effort level"

  Scenario: --effort flag is documented in help
    Given a temp base directory
    When I run quecto help
    Then the exit code should be 0
    And stdout should contain "--effort"
