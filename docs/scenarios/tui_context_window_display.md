# Scenario: quecto-tui displays the actual context window instead of a hardcoded 200k heuristic

Given quecto-tui is connected to a QuEcto agent over UDS
When the agent reports session stats with input token usage and max context tokens
Then the footer should display context usage using the reported max context window
And it should not assume a fixed 200k context window

Given no context stats are available yet
When the footer renders
Then it should show an unknown percentage with the actual known window if available
Or `?/0` if no window is known yet
