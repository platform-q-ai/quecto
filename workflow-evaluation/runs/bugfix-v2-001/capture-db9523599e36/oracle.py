from settings import resolve
d = {"debug":True,"retries":3,"prefix":"x","host":"local","port":8000}
o = {"debug":False,"retries":0,"prefix":"","host":None,"unknown":1}
before_d, before_o = dict(d), dict(o)
result = resolve(d, o)
assert result == {"debug":False,"retries":0,"prefix":"","host":"local","port":8000}
assert result is not d and result is not o
assert d == before_d and o == before_o
assert resolve({}, {"x":0}) == {}
assert resolve({"x":None}, {}) == {"x":None}
print("falsy, None, missing, unknown and ownership checks pass")
