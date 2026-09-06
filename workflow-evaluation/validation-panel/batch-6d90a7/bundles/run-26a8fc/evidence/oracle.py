import json
from pathlib import Path
from report import total
rows = json.loads(Path("rows.json").read_text())
assert total(rows) == 5
assert sum(r["units"] for r in rows if r["status"].lower() == "paid") == 9
print("exact paid=5; case-insensitive counterfactual=9; current behavior conforms to SPEC")
