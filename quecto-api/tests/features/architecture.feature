@done @issue-1139 @architecture
Feature: Architecture boundaries
  As a maintainer
  I want clean architecture boundaries enforced
  So that the domain and application layers stay transport-agnostic

  Scenario: Domain layer has no outward imports
    Then the domain source should not import from infrastructure
    And the domain source should not import from application

  Scenario: Application layer has no infrastructure imports
    Then the application source should not import from infrastructure

  Scenario: Application layer uses ports, not concrete transport types
    Then the application source should not contain "UnixStream"
    And the application source should not contain "axum"
    And the application source should not contain "hyper"
    And the application source should not contain "tokio::net"
    And the application source should not contain "WebSocket"
