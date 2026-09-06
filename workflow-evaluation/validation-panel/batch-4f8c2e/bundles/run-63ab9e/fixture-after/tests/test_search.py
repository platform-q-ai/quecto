import unittest

from search import find


class SearchTests(unittest.TestCase):
    def test_default_and_none_preserve_order_and_duplicates(self):
        names = ["teapot", "coffee", "TEA", "teapot", "iced tea"]
        expected = ["teapot", "TEA", "teapot", "iced tea"]
        self.assertEqual(find(names, "TeA"), expected)
        self.assertEqual(find(names, "TeA", limit=None), expected)

    def test_limits_count_matches_including_duplicates(self):
        names = ["coffee", "Tea", "Tea", "water", "teapot"]
        for limit, expected in [
            (0, []),
            (1, ["Tea"]),
            (2, ["Tea", "Tea"]),
            (3, ["Tea", "Tea", "teapot"]),
            (10, ["Tea", "Tea", "teapot"]),
        ]:
            with self.subTest(limit=limit):
                self.assertEqual(find(names, "TEA", limit=limit), expected)

    def test_negative_limits_raise_even_without_matches(self):
        for names in ([], ["coffee"], ["Tea"]):
            for limit in (-1, -10):
                with self.subTest(names=names, limit=limit):
                    with self.assertRaises(ValueError):
                        find(names, "tea", limit=limit)

    def test_inputs_remain_unchanged(self):
        names = ["Tea", "coffee", "Tea", "teapot"]
        original = names.copy()
        query = "TEA"
        for limit in (None, 0, 1, 10):
            with self.subTest(limit=limit):
                result = find(names, query, limit=limit)
                result.append("new name")
                self.assertEqual(names, original)
                self.assertEqual(query, "TEA")

    def test_empty_inputs_and_no_matches(self):
        self.assertEqual(find([], "tea", limit=2), [])
        self.assertEqual(find(["coffee"], "tea", limit=2), [])
        self.assertEqual(find(["Tea", "coffee", "Tea"], "", limit=2),
                         ["Tea", "coffee"])

    def test_casefold_matching_with_limit(self):
        self.assertEqual(find(["Straße", "STRASSE"], "strasse", limit=1),
                         ["Straße"])
