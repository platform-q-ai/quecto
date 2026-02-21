@pending
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
    And a mock Telegram API

  # --- Happy path ---

  Scenario: Voice message is transcribed and processed by the agent
    Given Groq voice transcription is configured with api_key "gsk-test-key"
    And a mock Groq Whisper API that returns transcription "Turn off the lights"
    And the mock LLM returns a text response "Lights turned off"
    When user "12345" sends a voice message via Telegram to the running gateway
    Then the gateway should download the voice file from Telegram
    And the audio should be sent to the Groq Whisper API
    And the mock LLM should have received a request containing "Turn off the lights"
    And the Telegram API should have received a sendMessage to chat "12345"
    And the sent message should contain "Lights turned off"

  # --- No API key ---

  Scenario: Voice message is rejected when Groq API key is not configured
    Given no Groq API key is configured
    When user "12345" sends a voice message via Telegram to the running gateway
    Then the Telegram API should have received a sendMessage to chat "12345"
    And the sent message should contain "voice transcription is not configured"
    And the mock LLM should not have received any requests

  # --- Transcription error ---

  Scenario: Groq API error sends friendly message to user
    Given Groq voice transcription is configured with api_key "gsk-test-key"
    And a mock Groq Whisper API that returns an HTTP 500 error
    When user "12345" sends a voice message via Telegram to the running gateway
    Then the Telegram API should have received a sendMessage to chat "12345"
    And the sent message should contain an error indication
    And the mock LLM should not have received any requests

  # --- Voice alongside text ---

  Scenario: Gateway handles text and voice messages in the same session
    Given Groq voice transcription is configured with api_key "gsk-test-key"
    And a mock Groq Whisper API that returns transcription "What time is it"
    And the mock LLM returns a text response "It is 3pm"
    When user "12345" sends text "Hello" via Telegram to the running gateway
    And user "12345" sends a voice message via Telegram to the running gateway
    Then the mock LLM should have received a request containing "Hello"
    And the mock LLM should have received a request containing "What time is it"
