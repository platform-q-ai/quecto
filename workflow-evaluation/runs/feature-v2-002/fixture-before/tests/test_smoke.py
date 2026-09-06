import subprocess
import sys
import unittest
class Smoke(unittest.TestCase):
    def test_default(self):
        result = subprocess.run([sys.executable,"-B","linecount.py"], input="a\nb\n", text=True, capture_output=True)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "lines=2\n")
