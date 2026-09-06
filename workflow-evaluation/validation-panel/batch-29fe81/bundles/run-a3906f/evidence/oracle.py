import copy
from rollup import by_category, total
rows = [{"category":"food","amount":3},{"category":"travel","amount":4},
        {"category":"food","amount":-3},{"amount":2},{"category":"","amount":5},
        {"category":"uncategorized","amount":-1}]
before = copy.deepcopy(rows)
result = by_category(rows)
assert list(result.items()) == [("food",0),("travel",4),("uncategorized",1),("",5)]
assert by_category([]) == {} and total(rows) == 10
assert rows == before
print("category aggregation, order, zero totals, defaults and ownership pass")
