import csv, subprocess, sys, tempfile
from pathlib import Path
doc = Path("README.md").read_text()
assert "Export item records as a CSV file for spreadsheet users." in doc
assert "Consumers rely on the id,name column order." in doc
for old in ("--input", "--output", "--separator", "default semicolon"):
    assert old not in doc, old
for new in ("--dest", "--delimiter", "data/items.json", "build/items.csv"):
    assert new in doc, new
# Extract the actual documented example rather than substitute a correct command.
import shlex
lines = [line.strip().strip("`") for line in doc.splitlines()]
examples = [shlex.split(line) for line in lines if line.startswith("python3 ") and "export.py" in line]
assert examples, "expected a standalone python3 export.py example"
cmd = examples[0]
assert cmd[:2] == ["python3", "export.py"] and "data/items.json" in cmd
assert cmd[cmd.index("--dest") + 1] == "build/items.csv"
assert "--delimiter" not in cmd or cmd[cmd.index("--delimiter") + 1] == ","
with tempfile.TemporaryDirectory() as tmp:
    cmd[0] = sys.executable
    cmd[cmd.index("--dest") + 1] = str(Path(tmp) / "items.csv")
    subprocess.run(cmd, check=True, timeout=10, capture_output=True)
    with (Path(tmp) / "items.csv").open(newline="") as f:
        assert list(csv.reader(f)) == [["id", "name"], ["1", "tea"], ["2", "red, blue"]]
print("documented example exports correct CSV; panel checks option descriptions")
