import unittest

from receipts import total_cents


class TotalCentsTests(unittest.TestCase):
    def test_float_rounding_regressions(self):
        for amount, expected in [("0.29", 29), ("0.57", 57), ("1.15", 115)]:
            with self.subTest(amount=amount):
                self.assertEqual(total_cents([amount]), expected)
        self.assertEqual(total_cents(["0.29"] * 100), 2900)

    def test_mixed_fractional_lengths(self):
        self.assertEqual(total_cents(["12", "3.4", "0.05"]), 1545)

    def test_empty(self):
        result = total_cents([])
        self.assertEqual(result, 0)
        self.assertIsInstance(result, int)

    def test_boundaries(self):
        for amount, expected in [
            ("0", 0),
            ("0.0", 0),
            ("0.00", 0),
            ("0.01", 1),
            ("999999.99", 99999999),
            ("1000000", 100000000),
            ("1000000.0", 100000000),
            ("1000000.00", 100000000),
        ]:
            with self.subTest(amount=amount):
                result = total_cents([amount])
                self.assertEqual(result, expected)
                self.assertIsInstance(result, int)

    def test_total_can_exceed_single_amount_limit(self):
        self.assertEqual(total_cents(["1000000", "1000000"]), 200000000)
