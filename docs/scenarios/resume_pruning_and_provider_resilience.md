# Resume pruning and provider resilience

## Scenario: resumed sessions are pruned before the first model request

Given a saved session contains enough historical messages to exceed the configured context budget
When the user resumes that session and sends a new prompt
Then quecto prunes/collapses the loaded history before constructing the first chat request
And the provider is not asked to handle an over-context prompt that could have been pruned locally.

## Scenario: retryable provider failures are retried before surfacing an error

Given an LLM provider returns a transient network/server/rate-limit failure
When quecto sends a chat request
Then quecto retries the request with bounded backoff
And only surfaces an error if the retry budget is exhausted.

## Scenario: context/output-limit failures include actionable diagnostics

Given an LLM provider rejects a request because the prompt plus requested output exceeds model limits
When quecto reports the provider failure
Then the error message identifies it as a context/output limit issue
And suggests reducing prompt history, max output tokens, or enabling/prioritizing pruning.
