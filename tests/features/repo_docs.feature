@docs @wip
Feature: Repository documentation
  As a maintainer
  I want repository metadata in docs to match the actual private-repo status
  So that users are not told the project is MIT-licensed when it is proprietary

  @docs
  Scenario: README license section documents the private proprietary license
    When I read the repository file "README.md"
    Then the output should contain "## License"
    And the output should contain "LicenseRef-Proprietary"
    And the output should contain "private repository"
    And the output should not contain "## License\n\nMIT"
