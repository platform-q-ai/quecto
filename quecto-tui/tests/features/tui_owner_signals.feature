@tui @done @owner-signals
Feature: Termination signals to the TUI end its owned harness
  As a TUI user whose terminal closes, or who is signalled from outside
  I want the harness the TUI launched to end the way Ctrl+D ends it
  So that no owned agent is left running per closed terminal

  # #2053: SIGHUP, SIGTERM and an external SIGINT take the ordinary-exit
  # path. A real quecto-tui process owns a real stand-in harness (a script
  # found as `quecto` on PATH that announces a socket the scenario serves and
  # logs every signal it receives). SIGKILL cannot run any exit path: the
  # harness is armed to receive SIGTERM from the kernel on its parent's death
  # instead — unless the exit policy is detach-on-exit, which arms nothing.

  Scenario: SIGHUP to the TUI ends its owned harness through the ordinary exit
    Given a real TUI process owns a stand-in harness
    When the TUI process receives SIGHUP
    Then the stand-in harness should have exited on one SIGTERM within 10 seconds
    And the TUI process should have exited with code 0

  Scenario: SIGTERM to the TUI ends its owned harness through the ordinary exit
    Given a real TUI process owns a stand-in harness
    When the TUI process receives SIGTERM
    Then the stand-in harness should have exited on one SIGTERM within 10 seconds
    And the TUI process should have exited with code 0

  Scenario: SIGINT to the TUI ends its owned harness through the ordinary exit
    Given a real TUI process owns a stand-in harness
    When the TUI process receives SIGINT
    Then the stand-in harness should have exited on one SIGTERM within 10 seconds
    And the TUI process should have exited with code 0

  Scenario: SIGKILL to the TUI ends its owned harness by parent death
    Given a real TUI process owns a stand-in harness
    When the TUI process receives SIGKILL
    Then the stand-in harness should have exited on one SIGTERM within 10 seconds
    And the TUI process should have been killed by signal 9

  Scenario: Detach-on-exit leaves the owned harness running whatever ends the TUI
    Given a real TUI process owns a stand-in harness started with --detach-on-exit
    When the TUI process receives SIGKILL
    Then the stand-in harness should still be running 2 seconds later
    And the TUI process should have been killed by signal 9

  Scenario: A signal during the startup window ends the harness being started at once
    Given a real TUI process is starting a stand-in harness that announces its socket after 15 seconds
    When the TUI process receives SIGHUP
    Then the stand-in harness should have received its SIGTERM within 6 seconds of the signal
    And the TUI process should have exited with code 1

  Scenario: A signal during the startup window leaves a detach-on-exit harness running
    Given a real TUI process is starting a stand-in harness with --detach-on-exit that announces its socket after 3 seconds
    When the TUI process receives SIGTERM
    Then the TUI process should have exited with code 1
    And the stand-in harness should still be running 5 seconds later

  Scenario: An attached TUI leaving on SIGHUP starts and ends no harness
    Given a real TUI process is attached to a socket served by the scenario
    When the TUI process receives SIGHUP
    Then no stand-in harness was ever started
    And the TUI process should have exited with code 0
