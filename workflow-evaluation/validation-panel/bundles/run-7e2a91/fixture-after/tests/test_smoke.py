import unittest
from inventory import summarize
class Smoke(unittest.TestCase):
    def test_count(self):
        self.assertEqual(summarize(["a.txt","b.csv"]), {"text":1,"data":1})
