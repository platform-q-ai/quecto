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

  Scenario: A searched row shows its title, folder, ID, repository, match and folder state
    When I type "a" into the resume search box
    And the harness answers the search in flight with a scoped and an unscoped session
    Then the resume picker shows the row "Zebra cache" with "/work/alpha"
    And the resume picker details show "ID chat-1700000000-abc"
    And the resume picker details show "repo alpha · matched: title"
    When I press Down in the resume results
    Then the resume picker details show "No folder recorded (older session)"
    And the resume picker details show "ID cli:legacy"

  Scenario: An answer the user has typed past is progress and never settles the picker
    When I type "ze" into the resume search box
    Then the resume picker shows "Sessions · Searching…"
    And the resume picker shows "LISTED-ONE"
    When the harness answers the search in flight with the session "OVERTAKEN-Z-ROW"
    Then the resume picker shows "OVERTAKEN-Z-ROW"
    And the resume picker shows "Sessions · Searching…"
    And one metadata search is in flight for "ze" in scope "global"
    When the harness answers the search in flight with the session "FRESH-ZE-ROW"
    Then the resume picker shows "FRESH-ZE-ROW"
    And the resume picker does not show "Searching…"
    And no metadata search is in flight

  Scenario: Enter typed ahead of the answer resumes the answered row, never a listed one
    When I type "zebr" into the resume search box
    And I press Enter twice in the resume picker
    Then no resume request was sent
    When the harness answers the search in flight with the session "OVERTAKEN-Z-ROW"
    Then no resume request was sent
    And one metadata search is in flight for "zebr" in scope "global"
    When the harness answers the search in flight with the session "ZEBRA-PLAN"
    Then one resume request is sent for "cli:answered" carrying version "h1-00000000000000c1"
    And the resume picker is closed

  Scenario: Enter typed before the first listing arrives resumes nothing
    When I submit /resume again and press Enter twice before its listing arrives
    And the harness answers the session list in flight for "local" with 2 listed sessions
    Then no resume request was sent
    And the resume picker shows "LISTED-ONE"
    And the resume picker does not show "will open"

  Scenario: An owed Enter is on screen and a focus change withdraws it
    When I type "zebr" into the resume search box
    And I press Enter twice in the resume picker
    Then the resume picker shows "Sessions · Searching… ⏎ will open the top match"
    When I press Tab in the resume picker
    Then the resume picker does not show "will open"
    When the harness answers every search until "zebr" with a hostile row
    Then no resume request was sent

  Scenario: An owed Enter is withdrawn when the search is not answered in time
    When I type "z" into the resume search box
    And I press Enter twice in the resume picker
    And the search in flight is not answered in time
    Then the resume picker shows "Sessions · Searching…"
    And the resume picker does not show "will open"
    When the harness answers the search in flight with the session "LATE-ZEBRA"
    Then the resume picker shows "LATE-ZEBRA"
    And no resume request was sent

  Scenario: Enter typed ahead of an answer with no match resumes nothing
    When I type "q" into the resume search box
    And I press Enter twice in the resume picker
    And the harness answers the search in flight with no sessions
    Then no resume request was sent
    And the resume picker shows "No sessions match"
    And the resume picker shows "in All Folders"
    And the resume picker does not show "No items"

  Scenario: An answer cut at the limit says how many matched
    When I type "m" into the resume search box
    And the harness answers the search in flight with 3 of 5200 matches
    Then the resume picker shows "Showing 3 of 5,200 — keep typing to narrow"

  Scenario: A refused query says so where the rows would be
    When I type "r" into the resume search box
    And the harness refuses the search in flight with "query too long: 300 characters (at most 256 are searched)"
    Then the resume picker shows "Search refused: query too long: 300 characters"

  Scenario: A pasted key is searched whole
    When I paste "  chat-1700000000-abc  " into the resume search box
    Then one metadata search is in flight for "chat-1700000000-abc" in scope "global"

  Scenario: A paste with the focus on Sessions goes to the search box, first line only
    When I press Tab in the resume picker
    And I paste "cli:one\rcli:two\r" into the resume search box
    Then one metadata search is in flight for "cli:one" in scope "global"
    And the resume picker shows "Pasted the first line only"

  Scenario: A picker closed by a tab switch abandons its search
    When I type "a" into the resume search box
    And the picker is closed by a tab switch
    And a search answer for the closed picker arrives with the session "AFTER-SWITCH"
    Then the resume picker is closed
    And no toast is shown
    And nothing but metadata searches and lists was ever sent

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
    And the resume picker shows "Sessions · No answer"
    And the resume picker shows "Search did not answer — edit the text or change Scope to retry"

  Scenario: A harness without the command is told once and the box filters the listed rows
    When I type "two" into the resume search box
    And the harness rejects search_session_metadata as an unknown command
    Then a toast says "Search needs a newer quecto harness — restart the agent"
    And the resume picker shows "LISTED-TWO"
    And the resume picker does not show "LISTED-ONE"
    When I clear the resume search box
    And I type "one" into the resume search box
    Then the resume picker shows "Sessions · Loading…"
    When the harness answers the session list in flight for "global" with 2 listed sessions
    Then the resume picker shows "LISTED-ONE"
    And the resume picker does not show "LISTED-TWO"
    And exactly one metadata search was ever sent
