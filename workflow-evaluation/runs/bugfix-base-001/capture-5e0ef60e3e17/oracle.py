from intervals import merge
cases = [([], []), ([(1,2),(2,3)], [(1,2),(2,3)]),
         ([(5,7),(1,4),(3,6)], [(1,7)]), ([(1,8),(2,3)], [(1,8)]),
         ([(1,3),(1,3)], [(1,3)]), ([(-4,-2),(-2,0)], [(-4,-2),(-2,0)]),
         ([(8,9)], [(8,9)]), ([(1,4),(4,6),(3,5)], [(1,6)])]
for value, expected in cases:
    before = list(value)
    assert merge(value) == expected, (value, expected)
    assert value == before, "mutated input"
print("8 interval cases and input preservation pass")
