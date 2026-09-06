import inspect
from inventory import summarize
assert str(inspect.signature(summarize)) == "(names)"
cases = [([],[]), (["x.JSON","a.txt","b.csv","c.md","z"],[("other",2),("text",2),("data",1)]),
         (["a.tar.json",".txt","file.","MD"],[("data",1),("text",1),("other",2)])]
for names, expected in cases:
    before = list(names)
    assert list(summarize(names).items()) == expected
    assert names == before
from classification import classify_name
assert classify_name("a.txt") == "text" and classify_name("a.TXT") == "other"
print("module extraction API and behavior parity pass; panel checks dependency and rule ownership")
