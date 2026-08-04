@done @issue-1369
Feature: Container-backed spawn launch
  Container-backed spawn requests are admitted through the spawn tool, configured by named script sets, and launched by a container backend.

  # AC1
  Scenario: SpawnTool keeps local default when container is omitted
    When the parent asks SpawnTool to spawn agent "local-default" without a container field
    Then SpawnTool reaches the local launch path

  # AC1
  Scenario: SpawnTool keeps local default when container is false
    When the parent asks SpawnTool to spawn agent "local-false" with container false
    Then SpawnTool reaches the local launch path

  # Wiring coverage, not AC1
  Scenario: SpawnTool fails closed for container requests until a script runtime is configured
    Given the spawn tool has container-backed launching enabled
    When the parent asks SpawnTool to spawn agent "builder" in a new container using script "quecto-dev"
    Then SpawnTool rejects the container request without falling back to local launch

  # AC2
  Scenario: Default container script selection uses configured default
    Given container scripts define default "quecto-dev" and script "api-dev"
    When a new container spawn requests no script
    Then the launch configuration selects container script "quecto-dev"

  # AC2
  Scenario: Explicit container script selection overrides the default script set
    Given container scripts define default "quecto-dev" and script "api-dev"
    When a new container spawn requests script "api-dev"
    Then the launch configuration selects container script "api-dev"

  # AC2
  Scenario: Missing container script selection fails before create
    Given container scripts define no default and script "api-dev"
    When a new container spawn requests no script
    Then launch configuration fails before create with "container spawn requires container_scripts.default or container.container_script"

  # AC2
  Scenario: Unknown container script selection fails before create
    Given container scripts define default "quecto-dev" and script "api-dev"
    When a new container spawn requests script "missing-dev"
    Then launch configuration fails before create with "container script set 'missing-dev' is not configured"

  # AC2
  Scenario: Incomplete container script selection fails before create
    Given container scripts define default "broken-dev" with incomplete create command
    When a new container spawn requests no script
    Then launch configuration fails before create with "container script set 'broken-dev' is incomplete"

  # AC3
  Scenario: Omitted repo uses the parent repository for new container launch
    Given the parent repository is "/work/parent-repo"
    When a new container launch request omits repo
    Then the launch request uses repository "/work/parent-repo"

  # AC3
  Scenario: Explicit alternate repo is preserved for new container launch
    Given the parent repository is "/work/parent-repo"
    When a new container launch request specifies repo "/work/alternate-repo"
    Then the launch request uses repository "/work/alternate-repo"

  # Backend wiring coverage, not AC3
  Scenario: Container launch backend handles new container requests
    Given a container launch backend is configured with script "quecto-dev"
    When the parent launches agent "builder" in a new container through the backend
    Then the backend invokes the create script before exec
    And the backend records the launched container ref
