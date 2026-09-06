from receipts import total_cents
for values, expected in [([],0),(["0.29"],29),(["0.57","0.29"],86),
                          (["1.2","3","0.01"],421),(["1000000.00"],100000000),
                          (["0.01"] * 1000,1000),(["0","0.00"],0)]:
    before = list(values)
    actual = total_cents(values)
    assert type(actual) is int and actual == expected, (values, actual, expected)
    assert values == before
print("7 exact-currency cases pass")
