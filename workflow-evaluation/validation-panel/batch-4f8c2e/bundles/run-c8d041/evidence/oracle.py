from preview import resolve_port
assert resolve_port({"port": 9000}, {"PORT": "8123"}) == 8123
assert resolve_port({"port": 9000}, {}) == 9000
assert resolve_port({}, {}) == 8000
print("effective=8123; without launch override=9000; default=8000")
