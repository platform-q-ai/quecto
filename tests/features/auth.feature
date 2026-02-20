@done
Feature: Authentication
  As a user
  I want to authenticate with LLM providers via OAuth or API tokens
  So that I can securely access their APIs

  @pending
  Scenario: Login with paste token for Anthropic
    When I run quecto with arguments "auth login --provider anthropic"
    And I paste the token "sk-ant-test-token"
    Then the credential should be stored for "anthropic"
    And the auth method should be "token"

  @pending
  Scenario: Login with OAuth for OpenAI
    When I run quecto with arguments "auth login --provider openai"
    Then the output should initiate an OAuth flow

  @pending
  Scenario: Login with device code for headless environments
    When I run quecto with arguments "auth login --provider openai --device-code"
    Then the output should contain a device code URL
    And the output should contain a user code

  Scenario: Store and retrieve a credential
    Given a credential store
    When I store a token "sk-test-123" for provider "openai"
    Then the credential for "openai" should exist
    And the credential token should be "sk-test-123"

  Scenario: Check auth status with no credentials
    Given a credential store with no credentials
    When I check auth status
    Then the auth status should report no providers

  Scenario: Check auth status with valid credentials
    Given a credential store
    And a stored credential for "openai" with method "oauth"
    When I check auth status
    Then the auth status should include "openai"
    And the auth status for "openai" should be "active"

  Scenario: Check auth status with expired credentials
    Given a credential store
    And a stored credential for "anthropic" that is expired
    When I check auth status
    Then the auth status for "anthropic" should be "expired"

  Scenario: Remove a specific credential
    Given a credential store
    And a stored credential for "openai" with method "token"
    When I remove the credential for "openai"
    Then the credential for "openai" should not exist

  Scenario: Remove all credentials
    Given a credential store
    And a stored credential for "openai" with method "token"
    And a stored credential for "anthropic" with method "token"
    When I remove all credentials
    Then the credential for "openai" should not exist
    And the credential for "anthropic" should not exist

  # --- CLI auth commands ---

  Scenario: CLI auth login stores a pasted token for OpenAI
    Given a quecto base directory at a temporary path
    When I run quecto with arguments "auth login --provider openai --token sk-test-openai-key"
    Then the output should contain "stored"
    And the credential for "openai" should exist in the base directory

  Scenario: CLI auth login stores a pasted token for Anthropic
    Given a quecto base directory at a temporary path
    When I run quecto with arguments "auth login --provider anthropic --token sk-ant-test-key"
    Then the output should contain "stored"
    And the credential for "anthropic" should exist in the base directory

  Scenario: CLI auth logout removes a stored credential
    Given a quecto base directory at a temporary path
    And a stored credential for "openai" in the base directory
    When I run quecto with arguments "auth logout --provider openai"
    Then the output should contain "removed"
    And the credential for "openai" should not exist in the base directory

  Scenario: CLI auth logout for nonexistent provider is a no-op
    Given a quecto base directory at a temporary path
    When I run quecto with arguments "auth logout --provider openai"
    Then the output should contain "no credential"

  Scenario: CLI auth status shows active credentials
    Given a quecto base directory at a temporary path
    And a stored credential for "openai" with method "token" in the base directory
    And a stored credential for "anthropic" with method "oauth" in the base directory
    When I run quecto with arguments "auth status"
    Then the output should contain "openai"
    And the output should contain "active"
    And the output should contain "anthropic"

  Scenario: CLI auth status shows no providers when empty
    Given a quecto base directory at a temporary path
    When I run quecto with arguments "auth status"
    Then the output should contain "no credentials"

  Scenario: CLI auth status flags expired credentials
    Given a quecto base directory at a temporary path
    And a stored credential for "anthropic" that is expired in the base directory
    When I run quecto with arguments "auth status"
    Then the output should contain "anthropic"
    And the output should contain "expired"

  # --- Provider credential wiring ---

  Scenario: Gateway loads provider key from credential store
    Given a config with no API key for "openai"
    And a stored credential for "openai" with token "sk-from-store"
    When the gateway initializes providers
    Then the OpenAI provider should use API key "sk-from-store"

  Scenario: Gateway prefers credential store over config file
    Given a config with API key "sk-from-config" for "openai"
    And a stored credential for "openai" with token "sk-from-store"
    When the gateway initializes providers
    Then the OpenAI provider should use API key "sk-from-store"

  Scenario: Gateway falls back to config when credential store is empty
    Given a config with API key "sk-from-config" for "openai"
    And no stored credential for "openai"
    When the gateway initializes providers
    Then the OpenAI provider should use API key "sk-from-config"

  Scenario: Token expiry triggers re-auth notification
    Given a stored credential for "openai" that is expired
    When the gateway checks provider readiness
    Then the gateway should report "openai" needs re-authentication
