import unittest
from search import find
class Smoke(unittest.TestCase):
    def test_search(self):
        self.assertEqual(find(["Tea", "coffee", "teapot"], "TEA"), ["Tea", "teapot"])
