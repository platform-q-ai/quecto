import unittest
from rollup import total
class Smoke(unittest.TestCase):
    def test_total(self):
        self.assertEqual(total([{"amount":2},{"amount":-1}]), 1)
