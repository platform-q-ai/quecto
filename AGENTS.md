# Agent Development Instructions

- Adhere rigorously to Clean Architecture principles, applying them with the discipline and judgment expected of Uncle Bob.
- Use TDD and BDD best practices throughout implementation, following the red–green–refactor cycle.
- Code defensively and account for all possible code paths, including failure and edge cases.
- Use assertions along critical paths to validate invariants and ensure correctness.
- **ALWAYS use affirmative (allowlist) guards instead of negative (denylist) guards.**
