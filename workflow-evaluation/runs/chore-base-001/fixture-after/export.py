import argparse
import csv
import json
from pathlib import Path
def main():
    p = argparse.ArgumentParser()
    p.add_argument("source")
    p.add_argument("--dest", required=True)
    p.add_argument("--delimiter", default=",")
    args = p.parse_args()
    rows = json.loads(Path(args.source).read_text())
    target = Path(args.dest)
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open("w", newline="") as f:
        writer = csv.writer(f, delimiter=args.delimiter)
        writer.writerow(["id", "name"])
        writer.writerows((r["id"], r["name"]) for r in rows)
if __name__ == "__main__":
    main()
