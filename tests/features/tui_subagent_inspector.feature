Feature: Sub-agent inspector — agent-targeted message tail (#795)
  As the quecto-tui client backing the sub-agent inspection panel
  I want get_messages_tail to optionally target a specific sub-agent
  So that the panel can show each sub-agent's live output over the existing
  kernel UDS connection without spawning a new process

  # The full-screen master-detail panel behaviour (double-Up activation, the
  # list→detail→close focus machine, navigation, scrolling, no-flash rendering)
  # is exercised behaviourally by the quecto-tui `tui_harness` render tests and
  # the inspector component/state tests. These scenarios pin the one piece that
  # the kernel owns: the optional agent_id on the get_messages_tail command.

  Scenario: An agent-targeted tail request preserves the agent_id round-trip
    Given a get_messages_tail wire line targeting agent "worker"
    When the kernel parses the command
    Then the parsed command targets agent "worker"

  Scenario: A plain tail request leaves the agent_id absent (parent-targeted)
    Given a get_messages_tail wire line with no agent_id
    When the kernel parses the command
    Then the parsed command targets no agent

  # Per-turn streaming (#797): a child emits subagent_messages_appended on its
  # own stream; the parent monitor re-stamps it with the child's id and forwards
  # it so the TUI inspector updates turn-by-turn without polling.
  Scenario: A child's per-turn messages event is re-stamped with the child id
    Given a child subagent_messages_appended wire line
    When the parent monitor forwards it for agent "worker" under parent "root"
    Then the forwarded event targets agent "worker" with parent "root"
