@done
Feature: TUI master tool access management
  Users manage which tools the master agent may use from the TUI.

  Scenario: Opening tool management shows tool state and bulk shortcuts
    Given the TUI has a current tool catalogue
    When the user opens tool management
    Then the modal shows each available tool with its current enabled state
    And the modal help shows the enable-all and disable-all shortcuts

  Scenario: Tool access changes are sent for the master agent only
    Given the TUI has a current tool catalogue
    When the user changes tool access for a tool
    Then the TUI sends the updated master tool access configuration to the kernel
    And child-agent tool access is unchanged

  Scenario: Tool management can show tools in two columns
    Given the TUI has many available tools
    When the user opens tool management on a wide terminal
    Then the modal shows tools in two columns
    And filtering, selection state, navigation, and bulk shortcuts still apply to the visible tools
