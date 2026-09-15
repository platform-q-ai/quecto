Feature: Synchronize a client transcript over UDS (#1857)
  As a UDS client holding a ledger position
  I want to ask for the committed messages after my revision
  So that I converge on the transcript without re-reading its history

  @done @issue-1973 @issue-1857 @persist
  Scenario: A client behind the newest revision receives the committed messages after it
    Given a persisted UDS session containing three plain messages
    When a client syncs the current epoch from revision 1
    Then the sync response should carry the 2 newest messages in order, caught up at revision 3

  @done @issue-1973 @issue-1857 @persist
  Scenario: A client at the newest revision receives an empty caught-up delta
    Given a persisted UDS session containing three plain messages
    When a client syncs the current epoch from revision 3
    Then the sync response should carry the 0 newest messages in order, caught up at revision 3

  @done @issue-1973 @issue-1857 @persist
  Scenario: A client naming another epoch is told to resynchronise with the newest transcript page
    Given a persisted UDS session containing three plain messages
    When a client syncs epoch 7 from revision 0
    Then the sync response should be a resync page of 3 messages at revision 3

  @done @issue-1973 @issue-1857 @persist
  Scenario: A client before the first committed revision is told to resynchronise
    Given a persisted UDS session containing three plain messages
    When a client syncs the current epoch from revision 0
    Then the sync response should be a resync page of 3 messages at revision 3

  @done @issue-1973 @issue-1857 @persist
  Scenario: A sync whose revision is not a number is refused as a parse error
    Given a persisted UDS session containing three plain messages
    When a client sends a sync whose revision is the string "one"
    Then the request should be refused with error "parse error"

  @done @issue-1973 @issue-1857 @persist
  Scenario: A sync addressed to a child of a harness without sub-agents is refused
    Given a persisted UDS session containing three plain messages
    When an idle client sends a sync addressed to child "no-such-child"
    Then the request should be refused with error "no sub-agent registry available"
