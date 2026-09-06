decide(active, locked, admin, owner) returns (allowed, reason). Inputs are booleans. Existing policy is authoritative.

Runtime: Python 3.10+ standard library; no installation or network needed.
Existing smoke checks: python3 -B -m unittest discover -s tests -v
