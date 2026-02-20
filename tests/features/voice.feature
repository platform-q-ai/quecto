@done
Feature: Voice Transcription
  As a Telegram user
  I want to send voice messages to Quecto
  So that I can interact hands-free

  Scenario: Transcribe audio with Groq Whisper client
    Given a Groq Whisper client with api_key "gsk-test-key"
    And a mock Whisper API that returns transcription "Hello world"
    When the whisper client transcribes audio
    Then the transcription result should be "Hello world"

  Scenario: Transcription fails when no API key configured
    Given a Groq Whisper client with no api_key
    When the whisper client transcribes audio
    Then the transcription should fail with "api key not configured"

  Scenario: Transcription error from API is handled gracefully
    Given a Groq Whisper client with api_key "gsk-test-key"
    And a mock Whisper API that returns an error
    When the whisper client transcribes audio
    Then the transcription should fail with an error message

  @pending
  Scenario: Transcribe a Telegram voice message end-to-end
    Given a running gateway with Telegram enabled
    And Groq voice transcription is configured with api_key "gsk-test"
    When user sends a voice message via Telegram
    Then the voice file should be downloaded
    And the voice file should be sent to Groq Whisper API
    And the transcribed text should be routed to the agent

  @pending
  Scenario: Voice transcription disabled when no Groq key
    Given a running gateway with Telegram enabled
    And no Groq API key is configured
    When user sends a voice message via Telegram
    Then the voice message should be ignored or a fallback response sent

  @pending
  Scenario: Transcription error in gateway is handled gracefully
    Given a running gateway with Telegram enabled
    And Groq voice transcription is configured
    And the Groq API returns an error
    When user sends a voice message via Telegram
    Then an error message should be sent to the user
    And the gateway should continue running
