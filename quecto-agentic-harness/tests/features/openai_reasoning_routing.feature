@done @providers @issue-1066
Feature: OpenAI endpoint and reasoning-effort rules (both auth modes)
  As a user driving OpenAI reasoning models with function tools
  I want the kernel to route per OpenAI's documented endpoint rules
  So that every built-in OpenAI model completes agent turns under both auth modes
  and the full documented reasoning-effort scale is configurable and honoured.

  # --- Endpoint routing ---
  # OpenAI rejects reasoning models combined with function tools on
  # /v1/chat/completions ("Function tools with reasoning_effort are not
  # supported ... Please use /v1/responses instead"). Reasoning models must be
  # routed to the Responses API regardless of auth mode.

  Scenario Outline: Reasoning model with tools over API-key auth completes via the Responses API
    Given a temp base directory
    And OpenAI's Chat Completions endpoint rejects reasoning models combined with function tools
    And an agent provider configured with an OpenAI API key
    When I send an agent turn with tools for model "openai-api/<model>"
    Then the agent turn should complete without an HTTP 400
    And the turn should have been served via the "Responses" endpoint
    And no request should have reached the "Chat Completions" endpoint

    Examples:
      | model         |
      | gpt-5.6-sol   |
      | gpt-5.6-terra |
      | gpt-5.6-luna  |

  Scenario: API-key-authenticated Responses requests carry no OAuth account identity
    Given a temp base directory
    And OpenAI's Chat Completions endpoint rejects reasoning models combined with function tools
    And an agent provider configured with an OpenAI API key
    When I send an agent turn with tools for model "openai-api/gpt-5.6-sol"
    Then the Responses request should authenticate with the API key only

  Scenario: OAuth-authenticated reasoning turns keep working with the ChatGPT account identity
    Given a temp base directory
    And OpenAI's Chat Completions endpoint rejects reasoning models combined with function tools
    And an agent provider configured with ChatGPT OAuth credentials
    When I send an agent turn with tools for model "openai-oauth/gpt-5.6-sol"
    Then the agent turn should complete without an HTTP 400
    And the turn should have been served via the "Responses" endpoint
    And the Responses request should carry the ChatGPT account identity

  Scenario: Non-reasoning model with tools over API-key auth stays on Chat Completions
    Given a temp base directory
    And OpenAI's Chat Completions endpoint accepts agent turns with tools
    And an agent provider configured with an OpenAI API key
    When I send an agent turn with tools for model "openai-api/gpt-5.5"
    Then the agent turn should complete without an HTTP 400
    And the turn should have been served via the "Chat Completions" endpoint
    And no request should have reached the "Responses" endpoint

  Scenario: Third-party openai-completions models stay on Chat Completions
    Given a temp base directory
    And OpenAI's Chat Completions endpoint accepts agent turns with tools
    And an agent provider configured with a third-party openai-completions endpoint "fireworks"
    When I send an agent turn with tools for model "fireworks/custom-model-x"
    Then the agent turn should complete without an HTTP 400
    And the turn should have been served via the "Chat Completions" endpoint
    And no request should have reached the "Responses" endpoint

  # --- Effort vocabulary (OpenAI documented scale) ---
  # OpenAI documents: none, low, medium (server default), high, xhigh.
  # The kernel keeps "max" for Anthropic models, so the full accepted set is
  # none, low, medium, high, xhigh, max.

  Scenario Outline: Every OpenAI-documented effort level is accepted at configuration time
    Given a temp base directory
    When I run the agent CLI with effort "<effort>"
    Then the CLI should accept the effort level "<effort>"

    Examples:
      | effort |
      | none   |
      | low    |
      | medium |
      | high   |
      | xhigh  |

  Scenario: Unknown effort string is rejected naming the valid values
    Given a temp base directory
    When I run the agent CLI with effort "turbo"
    Then the CLI should reject the effort level "turbo"
    And the error should name the valid effort values "none, low, medium, high, xhigh, max"
