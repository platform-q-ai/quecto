from search import find
names = ["Tea","coffee","teapot","Tea"]
before = list(names)
assert find(names, "TEA") == ["Tea","teapot","Tea"]
assert find(["Straße"], "STRASSE") == ["Straße"]
assert find(["Straße"], "STRASSE", limit=1) == ["Straße"]
assert find(names, "tea", limit=None) == ["Tea","teapot","Tea"]
assert find(names, "tea", limit=0) == []
assert find(names, "tea", limit=2) == ["Tea","teapot"]
assert find(names, "tea", limit=9) == ["Tea","teapot","Tea"]
assert find(names, "", limit=2) == names[:2]
assert find([], "", limit=2) == [] and find(names,"zzz",limit=1) == []
try:
    find(names, "tea", limit=-1)
except ValueError:
    pass
else:
    raise AssertionError("negative limit accepted")
assert names == before
print("limit contract and legacy search compatibility pass")
