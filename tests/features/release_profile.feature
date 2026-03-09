@done
Feature: Release profile and dependency hygiene
  As a maintainer
  I want Cargo.toml to include release profile tuning and lean dependencies
  So that release binaries are smaller and faster without manual configuration

  Scenario: Cargo.toml contains a release profile section
    Then Cargo.toml should contain a release profile with "opt-level = 3"
    And Cargo.toml should contain a release profile with "lto"
    And Cargo.toml should contain a release profile with "codegen-units = 1"
    And Cargo.toml should contain a release profile with "strip = true"
    And Cargo.toml should contain a release profile with 'panic = "abort"'

  Scenario: image crate is not a direct dependency
    Then Cargo.toml should not contain a direct dependency on "image"

  Scenario: serde_yaml is not a direct dependency
    Then Cargo.toml should not contain a direct dependency on "serde_yaml"
