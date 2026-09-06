import copy, html, inspect
from receipts import text_receipt, html_receipt
assert str(inspect.signature(text_receipt)) == "(items)"
assert str(inspect.signature(html_receipt)) == "(items)"
cases = [[],[("tea",1,20)],[("<&\\\"'",0,-3),("tea",2,100)],[("é",-1,0),("tea",1,0)]]
for items in cases:
    before = copy.deepcopy(items)
    lines = [f"{name}: {qty} {'item' if qty == 1 else 'items'} @ {cents}c" for name,qty,cents in items]
    assert text_receipt(items) == "\n".join(lines)
    assert html_receipt(items) == "<ul>" + "".join("<li>" + html.escape(line) + "</li>" for line in lines) + "</ul>"
    assert items == before
print("exact text/HTML parity and signatures pass; panel must inspect actual shared extraction")
