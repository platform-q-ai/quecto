import unittest

from search import find


class FindLimits(unittest.TestCase):
    def setUp(self):
        self.names = ["coffee", "Tea", "juice", "teapot", "Tea", "TEA"]
        self.original = self.names.copy()

    def tearDown(self):
        self.assertEqual(self.names, self.original)

    def test_none_preserves_matching_order_and_duplicates(self):
        expected = ["Tea", "teapot", "Tea", "TEA"]
        self.assertEqual(find(self.names, "TEA"), expected)
        self.assertEqual(find(self.names, "TEA", limit=None), expected)

    def test_positive_limits_count_matches_not_inputs(self):
        for limit, expected in [
            (1, ["Tea"]),
            (3, ["Tea", "teapot", "Tea"]),
            (4, ["Tea", "teapot", "Tea", "TEA"]),
            (10, ["Tea", "teapot", "Tea", "TEA"]),
        ]:
            with self.subTest(limit=limit):
                self.assertEqual(find(self.names, "TEA", limit=limit), expected)

    def test_zero_returns_empty(self):
        self.assertEqual(find(self.names, "tea", limit=0), [])

    def test_negative_limits_raise_even_without_matches(self):
        for names, query in [(self.names, "tea"), (self.names, "missing"), ([], "")]:
            with self.subTest(names=names, query=query):
                with self.assertRaises(ValueError):
                    find(names, query, limit=-1)

    def test_empty_inputs_and_no_matches(self):
        self.assertEqual(find([], "tea", limit=2), [])
        self.assertEqual(find(self.names, "missing", limit=2), [])

    def test_empty_query_matches_every_name(self):
        self.assertEqual(find(self.names, "", limit=3), self.names[:3])
