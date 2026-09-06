import unittest
from access import decide
class Smoke(unittest.TestCase):
    def test_admin(self):
        self.assertEqual(decide(True,False,True,False), (True,"admin"))
