Receipt rendering accepts a list of (name, quantity, cents) tuples. Names are strings; quantity/cents integers. Existing output is the compatibility contract.

Runtime: Python 3.10+ standard library; no installation or network needed.
Existing smoke checks: python3 -B -m unittest discover -s tests -v
