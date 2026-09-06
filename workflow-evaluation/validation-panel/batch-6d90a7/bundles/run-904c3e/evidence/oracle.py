from pathlib import Path
import re
p = Path("docs/start.md")
doc = p.read_text()
links = re.findall(r"\[([^]]+)\]\(([^)]+)\)", doc)
assert [label for label, _ in links] == ["installation", "troubleshooting"]
assert [(p.parent / target).resolve() for _, target in links] == [(p.parent / target).resolve() for target in ("guide/install.md", "support/troubleshooting.md")]
for _, target in links:
    assert (p.parent / target).is_file()
original_shape = "# Start\nUse [installation](LINK) first.\nFor help, see [troubleshooting](LINK).\nOffline use is supported.\n"
assert re.sub(r"\]\([^)]+\)", "](LINK)", doc) == original_shape
print("both relative links resolve; prose unchanged")
