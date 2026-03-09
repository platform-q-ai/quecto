@done
Feature: Release profile optimizations
  As a maintainer
  I want Cargo.toml to include release profile tuning
  So that release binaries are smaller and faster without manual configuration

  Scenario: Cargo.toml contains a release profile section
    Then Cargo.toml should contain a release profile with "opt-level = 3"
    And Cargo.toml should contain a release profile with "lto"
    And Cargo.toml should contain a release profile with "codegen-units = 1"
    And Cargo.toml should contain a release profile with "strip = true"
    And Cargo.toml should contain a release profile with 'panic = "abort"'
