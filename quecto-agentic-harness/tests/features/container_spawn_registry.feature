@done
Feature: Container spawn registry and shared environments

  Scenario: An observer joins a live environment and shares its checkout
    Given a parent session has created container ref "C1" for repository "https://github.com/platform-q-ai/quecto"
    When the parent spawns a read-only observer into existing container ref "C1"
    Then the observer is accepted into container ref "C1"
    And the observer workspace path matches the implementing agent workspace path

  Scenario: Unknown environment refs fail instead of guessing another target
    Given a parent session has created container ref "C1" for repository "https://github.com/platform-q-ai/quecto"
    When the parent spawns an agent into existing container ref "C99"
    Then the spawn fails because container ref "C99" is unknown
    And no other container is targeted

  Scenario: Dead environment refs fail instead of guessing another target
    Given a parent session has created container ref "C1" for repository "https://github.com/platform-q-ai/quecto"
    And container ref "C1" has stopped
    When the parent spawns an agent into existing container ref "C1"
    Then the spawn fails because container ref "C1" is not live
    And no other container is targeted

  Scenario: Environment refs are never reused after stop
    Given a parent session has created container ref "C1" for repository "https://github.com/platform-q-ai/quecto"
    And container ref "C1" has stopped
    When the parent creates another container for repository "https://github.com/platform-q-ai/quecto"
    Then the new container ref is "C2"

  Scenario: The parent protocol lists refs repositories and members
    Given a parent session has created container ref "C1" for repository "https://github.com/platform-q-ai/quecto"
    And the parent has spawned an implementer and observer in container ref "C1"
    When the parent requests the container list through the agent protocol
    Then the container list includes ref "C1"
    And the container list includes repository "https://github.com/platform-q-ai/quecto"
    And the container list includes the implementer and observer members

  Scenario: Agent identity is distinct from environment identity
    Given a parent session has created container ref "C1" for repository "https://github.com/platform-q-ai/quecto"
    And the parent has spawned an implementer and observer in container ref "C1"
    When the parent requests the container list through the agent protocol
    Then the container uuid is not the implementer agent uuid
    And the container uuid is not the observer agent uuid

  Scenario: Co-located agents report the same workspace path
    Given a parent session has created container ref "C1" for repository "https://github.com/platform-q-ai/quecto"
    And the parent has spawned an implementer and observer in container ref "C1"
    When the parent requests the container list through the agent protocol
    Then the implementer and observer have workspace path "/workspace/quecto"
