import unittest
from receipts import text_receipt
class Smoke(unittest.TestCase):
    def test_text(self):
        self.assertEqual(text_receipt([("tea",1,20)]), "tea: 1 item @ 20c")
