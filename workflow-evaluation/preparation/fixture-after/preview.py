import json
from pathlib import Path

def resolve_port(config, env):
    return int(env.get("PORT", config.get("port", 8000)))

if __name__ == "__main__":
    config = json.loads(Path("settings.json").read_text())
    env = json.loads(Path("launch.json").read_text())
    print(resolve_port(config, env))
