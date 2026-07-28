@issue-1282
Feature: Live child execution state
  As a parent agent supervising a child
  I want live, evidence-based execution state
  So that I can identify progress and decide whether intervention is needed

  @done
  Scenario: Running tool activity demonstrates recent progress
    Given a child agent is processing a turn with tool activity
    When the parent requests the child's state while a tool is running
    Then the state should identify the current execution phase and tool

  @done
  Scenario: Completed tools provide a bounded progress summary
    Given a child agent has completed tools during the current activity window
    When the parent requests the child's state
    Then the state should summarize recent completed and failed tool calls

  @done
  Scenario: In-flight message growth is visible to supervision
    Given a child agent appends conversation messages during an active turn
    When the parent requests the child's state before the turn completes
    Then the state message count should include the in-flight committed messages

  @done
  Scenario: Idle state clears transient execution activity
    Given a child agent has completed its active turn
    When the parent requests the child's state
    Then the state should report idle execution without a current tool

  @done
  Scenario: Parent agents receive distinct supervision and transcript guidance
    Given the agent command tool is available
    When a parent inspects its command guidance
    Then the guidance should distinguish live state from committed transcript history
