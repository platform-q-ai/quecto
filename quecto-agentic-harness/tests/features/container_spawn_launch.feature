@done
Feature: Container-backed spawn launch
  Container-backed spawn requests are admitted through the spawn tool, configured by named script sets, and launched by a container backend.

  # AC1
  Scenario: SpawnTool accepts a new container request instead of rejecting before launch
    Given the spawn tool has container-backed launching enabled
    When the parent asks SpawnTool to spawn agent "builder" in a new container using script "quecto-dev"
    Then SpawnTool accepts the container request for launch

  # AC2
  Scenario: Explicit container script selection overrides the default script set
    Given container scripts define default "quecto-dev" and script "api-dev"
    When a new container spawn requests script "api-dev"
    Then the launch configuration selects container script "api-dev"

  # AC3
  Scenario: Container launch backend handles new container requests
    Given a container launch backend is configured with script "quecto-dev"
    When the parent launches agent "builder" in a new container through the backend
    Then the backend invokes the create script before exec
    And the backend records the launched container ref
