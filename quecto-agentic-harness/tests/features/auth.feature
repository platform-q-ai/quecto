@done
Feature: Authentication
  As a user
  I want to authenticate with LLM providers via OAuth or API tokens
  So that I can securely access their APIs

  @done
  Scenario: Interactive login prompts for token paste
    Given a quecto base directory at a temporary path
    And a mock OAuth server for "anthropic" with token exchange
    When I start quecto with arguments "auth login --provider anthropic"
    And I paste the token "test-auth-code-123"
    Then the output should contain "stored"
    And the credential for "anthropic" should exist in the base directory
    And the stored credential method should be "oauth"

  @done
  Scenario: OAuth login initiates browser flow
    Given a quecto base directory at a temporary path
    And a mock OAuth server for "openai" with token exchange
    When I start quecto with arguments "auth login --provider openai --oauth"
    And I paste the token "test-auth-code-456"
    Then the output should contain a URL to open in the browser
    And the output should contain "stored"

  @done
  Scenario: Device code login for headless environments
    Given a quecto base directory at a temporary path
    And a mock OAuth server for "openai" supporting device code flow
    When I run quecto with arguments "auth login --provider openai --device-code"
    Then the output should contain a device code URL
    And the output should contain a user code to enter
    And the output should contain "Waiting for authorization"

  Scenario: Store and retrieve a credential
    Given a credential store
    When I store a token "sk-test-123" for provider "openai"
    Then the credential for "openai" should exist
    And the credential token should be "sk-test-123"

  Scenario: Existing credentials remain available until an update is complete
    Given a credential store
    And a stored credential for "openai" with token "old-token"
    When a credential update for "anthropic" is prepared but not completed
    Then the credential for "openai" should exist
    And the credential token should be "old-token"
    And the credential for "anthropic" should not exist


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

  # --- Gateway OAuth token refresh (issue #254) ---

  @done
  Scenario: Gateway refreshes expired OAuth token at startup
    Given a config with no API key for "anthropic"
    And a stored expired OAuth credential for "anthropic" with refresh token "rt-old-refresh"
    And a mock OAuth refresh server that returns a new token "sk-ant-oat01-new-access"
    When the gateway resolves API key with refresh for "anthropic"
    Then the resolved API key should be "sk-ant-oat01-new-access"
    And the persisted credential for "anthropic" should have token "sk-ant-oat01-new-access"

  @done
  Scenario: Gateway falls back to config key when OAuth refresh fails
    Given a config with API key "sk-ant-config-key" for "anthropic"
    And a stored expired OAuth credential for "anthropic" with refresh token "rt-bad-refresh"
    And a mock OAuth refresh server that returns an error
    When the gateway resolves API key with refresh for "anthropic"
    Then the resolved API key should be "sk-ant-config-key"

  @done
  Scenario: Gateway uses valid non-expired OAuth token without refresh
    Given a config with no API key for "anthropic"
    And a stored valid OAuth credential for "anthropic" with token "sk-ant-oat01-valid"
    When the gateway resolves API key with refresh for "anthropic"
    Then the resolved API key should be "sk-ant-oat01-valid"

  @done
  Scenario: Gateway async refresh updates credential store on disk
    Given a config with no API key for "openai"
    And a stored expired OAuth credential for "openai" with refresh token "rt-openai-refresh"
    And a mock OAuth refresh server that returns a new token "eyJnew-openai-token"
    When the gateway resolves API key with refresh for "openai"
    Then the resolved API key should be "eyJnew-openai-token"
    And the persisted credential for "openai" should have token "eyJnew-openai-token"

  # --- Optional refresh_token in OAuth response (issue #257) ---

  @done
  Scenario: OAuth refresh response without refresh_token preserves previous refresh token
    Given a config with no API key for "anthropic"
    And a stored expired OAuth credential for "anthropic" with refresh token "rt-original"
    And a mock OAuth refresh server that omits refresh_token and returns token "sk-ant-oat01-no-rt"
    When the gateway resolves API key with refresh for "anthropic"
    Then the resolved API key should be "sk-ant-oat01-no-rt"
    And the persisted credential for "anthropic" should have token "sk-ant-oat01-no-rt"
    And the persisted credential for "anthropic" should have refresh token "rt-original"

  @done
  Scenario: OAuth refresh response with new refresh_token updates stored refresh token
    Given a config with no API key for "anthropic"
    And a stored expired OAuth credential for "anthropic" with refresh token "rt-old"
    And a mock OAuth refresh server that returns a new token "sk-ant-oat01-refreshed" with refresh token "rt-new"
    When the gateway resolves API key with refresh for "anthropic"
    Then the resolved API key should be "sk-ant-oat01-refreshed"
    And the persisted credential for "anthropic" should have refresh token "rt-new"

  @done
  Scenario: OAuth token exchange response without refresh_token deserializes successfully
    Given a mock OAuth token exchange server that omits refresh_token
    When an OAuth token exchange is performed
    Then the token exchange should succeed with access token "sk-ant-oat01-exchanged"
    And the token exchange response should have no refresh token

  # --- Consistent expires_at safety margin (issue #256) ---

  @done
  Scenario: expires_at_with_margin applies consistent 5-minute safety buffer
    Given an OAuth token with expires_in of 3600 seconds
    When expires_at_with_margin is calculated
    Then the resulting expires_at should be 3300 seconds from now

  @done
  Scenario: Gateway refresh stores credential with safety margin
    Given a config with no API key for "anthropic"
    And a stored expired OAuth credential for "anthropic" with refresh token "rt-margin-test"
    And a mock OAuth refresh server that returns a new token "sk-ant-oat01-margin" with refresh token "rt-new"
    When the gateway resolves API key with refresh for "anthropic"
    Then the persisted credential for "anthropic" should have expires_at with 300-second safety margin for 28800 seconds

  @done
  Scenario: expires_at_with_margin handles zero expiry
    Given an OAuth token with expires_in of 0 seconds
    When expires_at_with_margin is calculated
    Then the resulting expires_at should be -300 seconds from now

  # --- OpenAI OAuth import refresh (issue #258) ---

  @done
  Scenario: OpenAI import refreshes expired token
    Given an external auth.json with expired OpenAI OAuth credential
    And a mock OAuth refresh server that returns a new token "eyJrefreshed-openai" with refresh token "rt-new-openai"
    When the external credentials are imported
    Then the stored OpenAI credential should have token "eyJrefreshed-openai"
    And the import output should contain "OpenAI token expired, refreshing"

  @done
  Scenario: OpenAI import stores non-expired token as-is
    Given an external auth.json with valid OpenAI OAuth credential "eyJvalid-openai"
    When the external credentials are imported
    Then the stored OpenAI credential should have token "eyJvalid-openai"

  @done
  Scenario: OpenAI import fails gracefully when refresh fails
    Given an external auth.json with expired OpenAI OAuth credential
    And a mock OAuth refresh server that returns an error
    When the external credentials are imported
    Then the import output should contain "failed to refresh OpenAI token"

  # --- Mid-session OAuth token refresh on 401 (issue #255) ---

  @done
  Scenario: RefreshableProvider refreshes token and retries on 401
    Given an OAuth-backed provider that returns 401 on first call
    And a mock OAuth refresh server that returns a new token "sk-ant-oat01-fresh"
    And the provider returns success after token refresh
    When a chat request is sent through the refreshable provider
    Then the request should succeed with the refreshed token
    And the credential store should contain the refreshed token

  @done
  Scenario: RefreshableProvider passes through non-401 errors unchanged
    Given an OAuth-backed provider that returns 500
    When a chat request is sent through the refreshable provider
    Then the request should fail with a server error

  @done
  Scenario: RefreshableProvider passes through successful responses
    Given an OAuth-backed provider that returns success
    When a chat request is sent through the refreshable provider
    Then the request should succeed normally

  @done
  Scenario: RefreshableProvider forwards requests without deep-cloning on happy path
    Given an OAuth-backed provider that returns success
    When a chat request is sent through the refreshable provider
    Then the request should succeed normally
    # Zero-copy pointer equality is verified by unit test
    # test_refreshable_forwards_without_cloning_on_happy_path
