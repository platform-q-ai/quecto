import unittest
from receipts import total_cents
class Smoke(unittest.TestCase):
    def test_integer(self):
        self.assertEqual(total_cents(["1", "2"]), 300)
