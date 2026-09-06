import unittest
from settings import resolve
class Smoke(unittest.TestCase):
    def test_override(self):
        self.assertEqual(resolve({"port":8000}, {"port":9000}), {"port":9000})
