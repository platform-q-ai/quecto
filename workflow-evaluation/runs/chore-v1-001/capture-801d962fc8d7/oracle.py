import json
from pathlib import Path
from loader import validate
config = json.loads(Path("config/example.json").read_text())
assert type(config["debug"]) is bool
assert validate(config) == {"host":"127.0.0.1", "port":8080, "debug":False,
                            "logging":{"level":"info", "format":"json"}}
print("example conforms to loader and local policy")
