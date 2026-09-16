@issue-2001 @issue-2001-safe-resume
Feature: Safe folder-aware resume through the public UDS protocol
  Exact opaque-key lookup is global, but only the same canonical execution
  directory/worktree may resume immediately. Every other transition is an
  explicit, affirmative, state-bound decision and is failure-atomic.

  Background:
    Given a temp base directory
    And a config file with an OpenAI provider pointing at a mock server
    And the mock LLM returns a text response "fixture saved"
    And the current execution folder is "project-a"

  @done @red @same-directory
  Scenario: The same canonical execution directory resumes immediately
    Given the real agent runtime saves session "same" from execution folder "project-a"
    When I request exact resume "cli:same" without choosing a disposition
    Then the resume plan should immediately resume opaque key "cli:same"

  @done @red @foreign @planning
  Scenario: A session in another execution directory requires the exact foreign allowlist
    Given the real agent runtime saves session "foreign" from execution folder "project-b"
    And the current execution folder is "project-a"
    When I request exact resume "cli:foreign" without choosing a disposition
    Then the resume plan should require exactly the choices "open_original,fork_current,cancel"
    And planning should preserve the active session key and history

  @done @red @foreign @open-original
  Scenario: Open original is attachably ready and enterable in the target folder
    Given the real agent runtime saves session "foreign" from execution folder "project-b"
    And the current execution folder is "project-a"
    When I request exact resume "cli:foreign" and choose "open_original"
    Then open original should be attachably ready in execution folder "project-b"

  @done @red @foreign @open-original @target-tools
  Scenario: Open original rediscovers target-folder configuration and tools
    Given the real agent runtime saves session "target-tools" from execution folder "project-b"
    And execution folder "project-b" enables folder-local web_fetch
    And the current execution folder is "project-a"
    When I request exact resume "cli:target-tools" and choose "open_original"
    Then open original should be attachably ready in execution folder "project-b"
    And the entered target runtime should rediscover tool "web_fetch"

  @done @red @foreign @fork @sanitization
  Scenario: Fork current creates a new safe transcript without runtime state
    Given execution folder "project-b" exists
    And legacy session "cli:unsafe" contains transcript and unsafe runtime state
    When I request exact resume "cli:unsafe" and choose "fork_current"
    Then the fork should use a new opaque key and retain only safe transcript state

  @done @red @foreign @cancel
  Scenario: Cancelling a foreign-folder decision consumes it without mutation
    Given the real agent runtime saves session "foreign" from execution folder "project-b"
    And the current execution folder is "project-a"
    When I request exact resume "cli:foreign", choose "cancel", and replay the decision
    Then resolution should succeed with status "cancelled"
    And planning should preserve the active session key and history
    And replaying the consumed decision should fail without another mutation

  @done @red @missing @planning
  Scenario: A moved or missing recorded folder exposes Locate Fork and Cancel
    Given the real agent runtime saves session "moved" from execution folder "project-b"
    And the current execution folder is "project-a"
    And execution folder "project-b" is removed
    When I request exact resume "cli:moved" without choosing a disposition
    Then the resume plan should require exactly the choices "locate,fork_current,cancel"
    And planning should preserve the active session key and history

  @done @red @missing @locate-current
  Scenario: Locate at the current directory returns a coherent same-directory outcome
    Given the real agent runtime saves session "moved" from execution folder "project-b"
    And execution folder "project-b" is removed
    And the current execution folder is "project-a"
    When I request exact resume "cli:moved" and choose locate at execution folder "project-a"
    Then locate should return a coherent same-directory resume outcome

  @done @red @missing @locate-elsewhere
  Scenario: Locate elsewhere returns a fresh foreign disposition without launching
    Given the real agent runtime saves session "moved" from execution folder "project-b"
    And execution folder "project-b" is removed
    And execution folder "project-c" exists
    And the current execution folder is "project-a"
    When I request exact resume "cli:moved" and choose locate at execution folder "project-c"
    Then locate should return a fresh foreign plan without an implicit launch

  @done @red @missing @fork
  Scenario: A missing-home session may be safely forked into current
    Given the real agent runtime saves session "moved" from execution folder "project-b"
    And execution folder "project-b" is removed
    And the current execution folder is "project-a"
    When I request exact resume "cli:moved" and choose "fork_current"
    Then resolution should succeed with status "forked"

  @done @red @missing @cancel
  Scenario: A missing-home decision may be cancelled without mutation
    Given the real agent runtime saves session "moved" from execution folder "project-b"
    And execution folder "project-b" is removed
    And the current execution folder is "project-a"
    When I request exact resume "cli:moved" and choose "cancel"
    Then resolution should succeed with status "cancelled"
    And planning should preserve the active session key and history

  @done @red @legacy @planning
  Scenario: A legacy unscoped session exposes associate safe fork and cancel
    Given a saved UDS session "cli:legacy" with 4 messages in the base directory
    When I request exact resume "cli:legacy" without choosing a disposition
    Then the resume plan should require the legacy choices "associate_current,fork_current,cancel"
    And planning should preserve the active session key and history

  @done @red @legacy @associate
  Scenario: Associate and resume here atomically associates a legacy record
    Given a saved UDS session "cli:legacy" with 4 messages in the base directory
    When I request exact resume "cli:legacy" and choose "associate_current"
    Then resolution should succeed with status "resumed"

  @done @red @legacy @fork
  Scenario: A legacy session supports safe fork
    Given a saved UDS session "cli:legacy" with 4 messages in the base directory
    When I request exact resume "cli:legacy" and choose "fork_current"
    Then resolution should succeed with status "forked"

  @done @red @legacy @cancel
  Scenario: A legacy decision supports cancellation without mutation
    Given a saved UDS session "cli:legacy" with 4 messages in the base directory
    When I request exact resume "cli:legacy" and choose "cancel"
    Then resolution should succeed with status "cancelled"
    And planning should preserve the active session key and history

  @done @red @allowlist
  Scenario: A choice absent from the affirmative allowlist is rejected
    Given the real agent runtime saves session "foreign" from execution folder "project-b"
    And the current execution folder is "project-a"
    When I request exact resume "cli:foreign" and choose "associate_current"
    Then resolution should fail without replacing the active conversation

  @done @red @replay
  Scenario: A completed decision is single-use
    Given the real agent runtime saves session "foreign" from execution folder "project-b"
    And the current execution folder is "project-a"
    When I request exact resume "cli:foreign", choose "fork_current", and replay the decision
    Then resolution should succeed with status "forked"
    And replaying the consumed decision should fail without another mutation

  @done @red @stale
  Scenario: A relevant active-state change invalidates the pending decision
    Given the real agent runtime saves session "foreign" from execution folder "project-b"
    And the current execution folder is "project-a"
    When I request exact resume "cli:foreign", change the active session, and choose "fork_current"
    Then the stale decision should fail after the active state changes

  @done @red @ownership-failure
  Scenario: Ownership conflict produces no false successful transition
    Given the real agent runtime saves session "foreign" from execution folder "project-b"
    And the current execution folder is "project-a"
    When I request exact resume "cli:foreign", lock its source after planning, and choose "fork_current"
    Then resolution should fail without replacing the active conversation

  @done @red @authoritative-write-failure
  Scenario: Authoritative fork write failure publishes no phantom destination
    Given a saved UDS session "cli:legacy" with 4 messages in the base directory
    When I request exact resume "cli:legacy" and fork while authoritative writes fail
    Then failed fork should publish no phantom destination and retain the current session

  @done @red @rollback-failure
  Scenario: Rollback cleanup failure is explicit and recoverable rather than false success
    Given a saved UDS session "cli:legacy" with 4 messages in the base directory
    When I request exact resume "cli:legacy" and fork while rollback cleanup also fails
    Then rollback cleanup failure should be reported explicitly with recovery information

  @done @red @launch-failure
  Scenario: Open failure after the recorded target disappears retains current state
    Given the real agent runtime saves session "foreign" from execution folder "project-b"
    And the current execution folder is "project-a"
    When I request exact resume "cli:foreign", remove its folder, and choose "open_original"
    Then resolution should fail without replacing the active conversation

  @done @red @readiness-failure @target-config
  Scenario: Target-folder configuration failure prevents premature Open success
    Given the real agent runtime saves session "foreign" from execution folder "project-b"
    And execution folder "project-b" has invalid folder-local configuration
    And the current execution folder is "project-a"
    When I request exact resume "cli:foreign" and choose "open_original"
    Then resolution should fail without replacing the active conversation

  @done @red @locate-validation
  Scenario: Locate never creates a missing operator-selected directory
    Given the real agent runtime saves session "moved" from execution folder "project-b"
    And execution folder "project-b" is removed
    And the current execution folder is "project-a"
    When I request exact resume "cli:moved" and choose locate at execution folder "does-not-exist"
    Then resolution should fail without replacing the active conversation
    And execution folder "does-not-exist" should remain absent
