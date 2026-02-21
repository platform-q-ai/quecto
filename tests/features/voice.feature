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

  @done
  Scenario: Telegram voice message is transcribed and routed to agent
    Given a running gateway with Telegram enabled and a mock Telegram API
    And Groq voice transcription is configured with api_key "gsk-test"
    And a mock Groq Whisper API that returns transcription "Turn off the lights"
    And a mock LLM provider
    When user "12345" sends a voice message via Telegram
    Then the gateway should download the voice file from Telegram
    And the voice file should be sent to the Groq Whisper API
    And the mock LLM should receive a request containing "Turn off the lights"

  @done
  Scenario: Voice message ignored when no Groq API key configured
    Given a running gateway with Telegram enabled and a mock Telegram API
    And no Groq API key is configured
    When user "12345" sends a voice message via Telegram
    Then the bot should respond to chat "12345" with "voice transcription is not configured"

  @done
  Scenario: Groq transcription error sends friendly error to user
    Given a running gateway with Telegram enabled and a mock Telegram API
    And Groq voice transcription is configured with api_key "gsk-test"
    And a mock Groq Whisper API that returns an HTTP 500 error
    When user "12345" sends a voice message via Telegram
    Then the bot should respond to chat "12345" with an error message
    And the gateway should continue processing other messages
