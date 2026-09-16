@done @issue-1996
Feature: Reasoning effort is a per-model capability that reaches the wire
  As a user of an OpenAI-compatible provider
  I want the effort I select to be sent to Fireworks and xAI
  So that the level shown in the UI is the level the model actually uses

  # ─── chat-completions request builder (#1996) ───────────────────────────

  Scenario Outline: A selected effort is serialised as reasoning_effort on the chat-completions request
    Given a chat-completions request for model "<model>" with effort "<effort>"
    When the "<provider>" OpenAI-compatible provider builds the chat-completions request
    Then the chat-completions body should set "reasoning_effort" to "<effort>"
    And the chat-completions body should not contain "thinking"

    Examples:
      | provider  | model                              | effort |
      | xai       | grok-4.6                           | xhigh  |
      | xai       | grok-4.5                           | high   |
      | fireworks | accounts/fireworks/models/glm-5p3  | low    |

  Scenario: No selected effort sends no reasoning option at all
    Given a chat-completions request for model "qwen3.6-35b-a3b-int4" with no effort
    When the "spark-local" OpenAI-compatible provider builds the chat-completions request
    Then the chat-completions body should not contain "reasoning_effort"
    And the chat-completions body should not contain "thinking"

  Scenario: OpenAI's own chat-completions endpoint never carries reasoning_effort
    # OpenAI rejects reasoning_effort with function tools on Chat Completions;
    # its reasoning ids are routed to the Responses API instead.
    Given a chat-completions request for model "gpt-5.5" with effort "high"
    When the "openai" OpenAI-compatible provider builds the chat-completions request
    Then the chat-completions body should not contain "reasoning_effort"

  # ─── the catalogue's per-model vocabulary drives every surface ──────────

  Scenario: A Fireworks model that declares reasoning advertises the common scale and the selection reaches the wire
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a models registry with reasoning Fireworks model "accounts/fireworks/models/glm-5p3"
    When I start the UDS agent with no session
    And I send set_model "fireworks/accounts/fireworks/models/glm-5p3"
    And I send set_effort "high"
    And I send prompt "think hard"
    And I send command "get_state" with id "gs-1"
    And I close the UDS connection
    Then the agent output should contain a response command "set_effort" with success true
    And the get_state response effort should be "high"
    And the get_state response effort levels should be "low, medium, high"
    And the Fireworks provider should have received a chat completion request with reasoning_effort "high"

  Scenario: A Fireworks model that declares no reasoning offers no effort control and sends none
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a models registry with Fireworks model "accounts/fireworks/models/plain"
    When I start the UDS agent with no session
    And I send set_effort "high"
    And I send set_model "fireworks/accounts/fireworks/models/plain"
    And I send set_effort "low"
    And I send prompt "just answer"
    And I send command "get_state" with id "gs-1"
    And I close the UDS connection
    Then the set_effort response should fail mentioning "no reasoning-effort control"
    And the get_state response effort should be unset
    And the get_state response effort levels should be ""
    And the Fireworks provider should have received a chat completion request without "reasoning_effort"

  Scenario: Changing effort between turns changes the next request's reasoning_effort
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a models registry with reasoning Fireworks model "accounts/fireworks/models/glm-5p3"
    When I start the UDS agent with no session
    And I send set_model "fireworks/accounts/fireworks/models/glm-5p3"
    And I send set_effort "low"
    And I send prompt "first"
    And I send set_effort "high"
    And I send prompt "second"
    And I close the UDS connection
    Then the Fireworks provider's chat completion requests should carry reasoning_effort "low, high" in order

  Scenario: An xAI level another Grok accepts is rejected for the active Grok, naming its own levels
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    When I start the UDS agent with no session
    And I send set_model "xai/grok-4.5"
    And I send set_effort "xhigh"
    And I send command "get_state" with id "gs-1"
    And I close the UDS connection
    Then the set_effort response should fail mentioning "valid levels: low, medium, high"
    And the get_state response effort levels should be "low, medium, high"

  Scenario: The startup effort flag is refused when the startup model does not accept it
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a models registry with Fireworks model "accounts/fireworks/models/plain"
    And the config default model is "fireworks/accounts/fireworks/models/plain"
    When I run quecto agent --effort high -m "hi"
    Then the exit code should be 1
    And stderr should contain "--effort"
    And stderr should contain "no reasoning-effort control"

  Scenario: A fresh session restores the startup effort only where the active model accepts it
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the config default model is "openai-api/gpt-5.6-sol"
    And the config default effort is "xhigh"
    When I start the UDS agent with no session
    And I send set_model "xai/grok-4.5"
    And I send set_effort "medium"
    And I send command "new_session" with id "ns-1"
    And I send command "get_state" with id "gs-1"
    And I close the UDS connection
    Then the get_state response effort should be unset
    And the get_state response effort levels should be "low, medium, high"

  Scenario: A fresh session restores the startup effort on the startup model
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the config default model is "openai-api/gpt-5.6-sol"
    And the config default effort is "xhigh"
    When I start the UDS agent with no session
    And I send set_effort "low"
    And I send command "new_session" with id "ns-1"
    And I send command "get_state" with id "gs-1"
    And I close the UDS connection
    Then the get_state response effort should be "xhigh"
