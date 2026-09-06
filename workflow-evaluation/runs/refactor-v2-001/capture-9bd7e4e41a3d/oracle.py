import inspect, itertools
from access import decide
assert str(inspect.signature(decide)) == "(active, locked, admin, owner)"
for active,locked,admin,owner in itertools.product((False,True), repeat=4):
    expected = ((False,"inactive") if not active else (False,"locked") if locked else
                (True,"admin") if admin else (True,"owner") if owner else (False,"not-owner"))
    actual = decide(active,locked,admin,owner)
    assert actual == expected and type(actual[0]) is bool
print("all 16 authorization combinations and exact reasons preserved; panel checks guard-clause structure")
