@done
Feature: End-to-End Gateway Voice Transcription
  As a Telegram user
  I want to send voice messages to Quecto through the gateway
  So that my spoken messages are transcribed and processed by the agent

  The gateway should detect voice messages in Telegram updates, download
  the audio file from Telegram's API, send it to the Groq Whisper API for
  transcription, and route the resulting text through the agent loop. These
  tests verify the full pipeline through the running gateway process.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And a mock Telegram API that supports voice downloads

  # --- Happy path ---

  Scenario: Voice message is transcribed and processed by the agent
    Given voice transcription is configured in the gateway config with api_key "gsk-test-key"
    And a mock Groq Whisper endpoint that returns transcription "Turn off the lights"
    And a mock LLM that captures requests and returns text "Lights turned off"
    When user "12345" sends a voice message via Telegram to the running gateway
    Then the Telegram mock should have received a getFile request
    And the captured LLM requests should contain "Turn off the lights"
    And the gateway Telegram mock should have received a sendMessage containing "Lights turned off"

  # --- No API key ---

  Scenario: Voice message is rejected when Groq API key is not configured
    Given no voice transcription API key is configured in the gateway config
    And a mock LLM that captures requests and returns text "should not be called"
    When user "12345" sends a voice message via Telegram to the running gateway
    Then the gateway Telegram mock should have received a sendMessage containing "voice transcription is not configured"
    And the captured LLM requests should be empty

  # --- Transcription error ---

  Scenario: Groq API error sends friendly message to user
    Given voice transcription is configured in the gateway config with api_key "gsk-test-key"
    And a mock Groq Whisper endpoint that returns an HTTP 500 error
    And a mock LLM that captures requests and returns text "should not be called"
    When user "12345" sends a voice message via Telegram to the running gateway
    Then the gateway Telegram mock should have received a sendMessage containing "could not transcribe"
    And the captured LLM requests should be empty

  # --- Voice alongside text ---

  Scenario: Gateway handles text and voice messages in the same session
    Given voice transcription is configured in the gateway config with api_key "gsk-test-key"
    And a mock Groq Whisper endpoint that returns transcription "What time is it"
    And a mock LLM that captures requests and returns text "It is 3pm"
    When user "12345" sends text "Hello" and then a voice message via Telegram to the running gateway
    Then the captured LLM requests should contain "Hello"
    And the captured LLM requests should contain "What time is it"
