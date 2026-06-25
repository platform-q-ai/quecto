Feature: Sub-agent session view + interaction parity, Tab focus model, focus divider (#802)
  As a human operator driving a workflow with sub-agents in the TUI
  I want a selected sub-agent to render its OWN full session (chat + workflow bar +
  footer gauges + spinner), to steer it anytime, and a NeoVim-ish two-pane focus
  model with a focus-highlighted divider
  So that selecting a sub-agent resumes that agent's own session rather than a
  different chat in the master shell

  # The acceptance criteria below are verified by the headless render harness
  # unit tests in quecto-tui/src/interface/app_focus_parity_tests.rs (focus
  # transitions, number-jump, per-session workflow/footer/spinner render,
  # send-routes-to-active-session, divider focus state). These scenarios document
  # the behaviour and are wired to steps in the GREEN phase; tagged @pending until
  # then so the gate is not blocked while the implementation lands.

  @pending
  Scenario: Selecting a sub-agent renders its own session chrome
    Given a TUI with a tracked sub-agent "w1" running a workflow
    When I select sub-agent "w1"
    Then the body renders w1's own chat
    And the body renders w1's own workflow/phase bar
    And the body renders w1's own footer context, cost and model
    And the body renders w1's own running spinner
    When I switch back to the master session
    Then the master's chat, workflow bar, footer and spinner are restored
    And each session keeps its own scroll and history

  @pending
  Scenario: Tab toggles focus between input and panel
    Given a TUI with at least one tracked sub-agent and no autocomplete popup open
    When I press Tab
    Then focus moves from the input to the side panel
    When I press Tab again
    Then focus returns to the input

  @pending
  Scenario: Tab keeps completing while an autocomplete popup is open
    Given a TUI with an open autocomplete popup
    When I press Tab
    Then the highlighted completion is accepted and focus does not move to the panel

  @pending
  Scenario: Panel focus moves highlight without connecting
    Given a TUI with tracked sub-agents and focus on the panel
    When I press Down or "j"
    Then the highlight moves but the active session does not change
    And no connect-on-select connection is opened on mere movement

  @pending
  Scenario: Digits jump the highlight to a numbered row
    Given a TUI with tracked sub-agents and focus on the panel
    When I press digit "2"
    Then the highlight jumps to panel row 2
    And the active session does not change until commit

  @pending
  Scenario: Enter commits the highlighted agent and connects
    Given a TUI with tracked sub-agents and focus on the panel
    When I move the highlight to a sub-agent and press Enter
    Then the active session switches to that sub-agent
    And a connect-on-select connection is opened for it
    And focus returns to the input

  @pending
  Scenario: Esc cancels panel focus without changing the selection
    Given a TUI viewing sub-agent "w1" with focus on the panel
    When I move the highlight and press Esc
    Then focus returns to the input
    And the active session is still "w1"

  @pending
  Scenario: Sending while a sub-agent is active steers that sub-agent
    Given a TUI viewing sub-agent "w1"
    When I type a prompt and send it
    Then the prompt is routed to w1's connection, not the master
    And the prompt is allowed even while w1 is mid-turn
    And a queued/working indicator shows until w1 processes it
    And w1's reply streams into w1's session
    And abort targets the active session

  @pending
  Scenario: Focus-highlighted divider between panel and body
    Given a TUI with a tracked sub-agent
    Then a vertical divider is drawn between the panel and the body
    And the divider is bright on the focused pane and dim on the other
    And the active/selected agent row is highlighted and shows its row number
