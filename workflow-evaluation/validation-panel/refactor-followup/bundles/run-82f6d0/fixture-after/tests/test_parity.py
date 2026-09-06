import unittest
from itertools import product

from access import decide


class DecideParity(unittest.TestCase):
    def test_all_boolean_combinations(self):
        # Rows follow product((False, True), repeat=4) in argument order:
        # active, locked, admin, owner. Outcomes capture the original policy.
        expected = [
            (False, "inactive"),
            (False, "inactive"),
            (False, "inactive"),
            (False, "inactive"),
            (False, "inactive"),
            (False, "inactive"),
            (False, "inactive"),
            (False, "inactive"),
            (False, "not-owner"),
            (True, "owner"),
            (True, "admin"),
            (True, "admin"),
            (False, "locked"),
            (False, "locked"),
            (False, "locked"),
            (False, "locked"),
        ]
        for arguments, outcome in zip(product((False, True), repeat=4), expected):
            with self.subTest(arguments=arguments):
                self.assertEqual(decide(*arguments), outcome)
