@workflow @reviewers
Feature: Feature workflow reviewers step hardening
  The feature workflow must dispatch PR reviewers only after a PR exists and must
  give reviewers the PR number so they fetch the diff themselves instead of
  receiving a raw diff blob.

  Scenario: Reviewers are given a PR number instead of a raw diff
    Given the feature workflow has an open pull request
    When the reviewers step dispatches sub-agents
    Then each reviewer should receive the PR number
    And each reviewer should fetch the diff with "gh pr diff <PR>"
    And the reviewers guidance should forbid passing a raw diff
    And each reviewer should post inline comments on the PR

  Scenario: Reviewers are not dispatched before a PR exists
    Given the feature workflow has no pull request yet
    When the reviewers step is reached
    Then the reviewers guidance should say not to dispatch reviewers before a PR exists
    And a precondition should deter reviewer dispatch without a PR

  Scenario: Reviewer hardening is mirrored in executable workflow sources
    Given the native feature workflow template is hardened for reviewer dispatch
    Then ".claude/workflows/feature.js" should carry the same reviewer dispatch hardening
    And workflow guard tests should assert the hardened wording
