@done @subagent-teardown @delegated-subtree @serial
Feature: A subtree ends by delegation and parent-loss, never by a pid from the root (#1940)

  A root harness owns only the children it launched, as handles. A grandchild
  it learns about from a child's snapshot is a routing target (uuid and
  launch generation) and nothing more: the root records no pid for it and
  never signals it. Killing the child ends the grandchild through the child's
  own acknowledged teardown; SIGKILL of the child ends the grandchild through
  the launch-bound parent-loss binding (#1935). Every wait is bounded.

  Background:
    Given a root harness whose "SPAWN_ONE" task makes a real child spawn a real grandchild

  Scenario: Killing a child with a grandchild ends the subtree by acknowledged delegation
    When the root spawns child "aye" with task "SPAWN_ONE"
    Then child "aye" reports grandchild "grand" with a launch generation and no pid
    When the root kills "aye" through its composed owner
    Then the kill of "aye" answered "graceful"
    And the processes of "aye" and its grandchild "grand" are gone within 30 seconds
    And the grandchild "grand" exited gracefully leaving no socket
    And the supervisor sent no signals to "aye"
    And every root row for "aye" and "grand" is exited

  Scenario: SIGKILL of the intermediate child ends its grandchild by parent loss
    When the root spawns child "aye" with task "SPAWN_ONE"
    Then child "aye" reports grandchild "grand" with a launch generation and no pid
    When the process of child "aye" is SIGKILLed behind the root's back
    Then the processes of "aye" and its grandchild "grand" are gone within 30 seconds
    And the grandchild "grand" exited gracefully leaving no socket
    And the supervisor sent no signals to "aye"
    And every root row for "aye" and "grand" is exited
