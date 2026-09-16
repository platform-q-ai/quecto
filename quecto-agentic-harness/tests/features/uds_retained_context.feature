@done @issue-1978 @issue-1866 @recall
Feature: Retained context over the real UDS agent loop
  As an operator whose agent collapses oversized tool output out of context
  I want the collapsed output retained under the session's identity and recalled by the real recall tool
  So that spill, index and recall are one capability's behaviour end to end, on disk

  Scenario: Collapsed tool output is indexed and recalled from the file retention store through the real loop
    Given a temp base directory
    And a real UDS agent for session "retained" over the file retention store that collapses after one tool result
    When the model runs the stub tool twice, then recalls the index, the collapsed id and an unknown id
    Then the recall index answered exactly the user prompt and both stub results in append order
    And the recall of the collapsed id answered the full stub output
    And the recall of the unknown id answered exactly "No spilled output found for id: turn9:stub:0" as an error
    And the first stub result is shown collapsed to the recall stub for "turn1:stub:0"
    And the file retention store of session "retained" holds "turn1:stub:0" and "turn2:stub:0" on disk
