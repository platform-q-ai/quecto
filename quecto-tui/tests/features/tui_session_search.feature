@tui @done @issue-2010
Feature: The /resume picker searches session metadata through the harness (#2010)
  As a TUI user looking for a session saved anywhere
  I want the search box to ask the harness for title, key, repository and path matches
  So that I find it without the TUI deciding scope, reading stores or acting on stale answers

  Background:
    Given a fresh TUI app harness
    And the resume picker is open on All Folders with 2 listed sessions

  Scenario: Typing asks the harness to search the scope on screen
    When I type "zeb" into the resume search box
    Then one metadata search is in flight for "z" in scope "global"
    When the harness answers the search in flight with no sessions
    Then one metadata search is in flight for "zeb" in scope "global"
    And no session list was requested by typing

  Scenario: A searched row shows its title, folder, key and unscoped state
    When I type "a" into the resume search box
    And the harness answers the search in flight with a scoped and an unscoped session
    Then the resume picker shows the row "Zebra cache" with "/work/alpha"
    And the resume picker details show "key chat-1700000000-abc"
    When I press Down in the resume results
    Then the resume picker details show "Unscoped · no folder on record"
    And the resume picker details show "key cli:legacy"

  Scenario: An answer to a query the user has typed past is discarded
    When I type "ze" into the resume search box
    And the harness answers the search in flight with the session "STALE-Z-ROW"
    Then the resume picker does not show "STALE-Z-ROW"
    And one metadata search is in flight for "ze" in scope "global"
    When the harness answers the search in flight with the session "FRESH-ZE-ROW"
    Then the resume picker shows "FRESH-ZE-ROW"
    And no metadata search is in flight

  Scenario: An answer with another generation or another id is discarded
    When I type "q" into the resume search box
    And the harness answers the search in flight under generation 0 with the session "WRONG-GENERATION"
    Then the resume picker does not show "WRONG-GENERATION"
    When a search answer with an unknown id arrives with the session "WRONG-ID"
    Then the resume picker does not show "WRONG-ID"

  Scenario: Switching to Local Folder searches locally and drops the global answer
    When I type "w" into the resume search box
    And I switch the resume picker to Local Folder
    And the harness answers the search in flight with the session "GLOBAL-ONLY-ROW"
    Then the resume picker does not show "GLOBAL-ONLY-ROW"
    And one metadata search is in flight for "w" in scope "local"
    When the harness answers the search in flight with the session "LOCAL-ROW"
    Then the resume picker shows "LOCAL-ROW"
    And the resume picker shows "[Local Folder]"

  Scenario: Clearing the search box lists the scope again
    When I type "w" into the resume search box
    And I clear the resume search box
    Then one session list is requested in scope "global"
    When the harness answers the search in flight with the session "LATE-SEARCH-ROW"
    Then the resume picker does not show "LATE-SEARCH-ROW"

  Scenario: Metacharacters are sent as typed and hostile metadata is shown safe
    When I type "a.*[b]" into the resume search box
    Then the latest metadata search asks for "a" literally
    When the harness answers every search until "a.*[b]" with a hostile row
    Then the latest metadata search asks for "a.*[b]" literally
    And the resume picker shows no control, bidi or zero-width character
    And the resume picker shows "evil title"

  Scenario: Picking a searched row asks the harness with the row's version
    When I type "a" into the resume search box
    And the harness answers the search in flight with a scoped and an unscoped session
    And I pick the second searched row
    Then one resume request is sent for "cli:legacy" carrying version "h1-00000000000000b2"

  Scenario: Escape during a search sends nothing more and drops the late answer
    When I type "a" into the resume search box
    And I press Escape in the resume picker
    And a search answer for the closed picker arrives with the session "AFTER-ESCAPE"
    Then the resume picker is closed
    And nothing but metadata searches and lists was ever sent

  Scenario: A failed search says so and keeps the rows on screen
    When I type "a" into the resume search box
    And the harness fails the search in flight with "sessions dir unreadable"
    Then a toast says "Could not search sessions"
    And the resume picker shows "LISTED-ONE"
