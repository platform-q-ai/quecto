import json
from pathlib import Path
def total(rows):
    return sum(r["units"] for r in rows if r["status"] == "paid")
if __name__ == "__main__":
    print(total(json.loads(Path("rows.json").read_text())))
