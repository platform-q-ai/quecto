import copy
import unittest

from rollup import by_category


class ByCategoryTests(unittest.TestCase):
    def test_sums_in_first_appearance_order(self):
        rows = [
            {"category": "travel", "amount": 5},
            {"category": "food", "amount": 3},
            {"category": "travel", "amount": 2},
            {"category": "books", "amount": 4},
            {"category": "food", "amount": 6},
        ]
        self.assertEqual(
            list(by_category(rows).items()),
            [("travel", 7), ("food", 9), ("books", 4)],
        )

    def test_missing_and_empty_categories(self):
        rows = [
            {"amount": 2},
            {"category": "", "amount": 3},
            {"category": "uncategorized", "amount": 4},
            {"amount": 5},
            {"category": "", "amount": 6},
        ]
        self.assertEqual(
            list(by_category(rows).items()),
            [("uncategorized", 11), ("", 9)],
        )

    def test_zero_totals_and_negative_amounts(self):
        rows = [
            {"category": "cancelled", "amount": 5},
            {"category": "negative", "amount": -3},
            {"category": "cancelled", "amount": -5},
            {"category": "zero", "amount": 0},
        ]
        self.assertEqual(
            list(by_category(rows).items()),
            [("cancelled", 0), ("negative", -3), ("zero", 0)],
        )

    def test_empty_input_returns_fresh_dict(self):
        result = by_category([])
        self.assertEqual(result, {})
        self.assertIsInstance(result, dict)
        self.assertIsNot(result, by_category([]))

    def test_does_not_mutate_rows_and_returns_independent_results(self):
        rows = [{"amount": 2}, {"category": "food", "amount": -1}]
        original = copy.deepcopy(rows)
        result = by_category(rows)
        self.assertEqual(rows, original)
        again = by_category(rows)
        self.assertIsNot(result, again)
        result["food"] = 100
        self.assertEqual(again, {"uncategorized": 2, "food": -1})
        self.assertEqual(rows, original)
