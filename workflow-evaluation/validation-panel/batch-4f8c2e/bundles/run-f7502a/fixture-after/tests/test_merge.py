import unittest

from intervals import merge


class MergeTests(unittest.TestCase):
    def test_touching_half_open_intervals_remain_separate(self):
        self.assertEqual(merge([(1, 3), (3, 5), (5, 7)]),
                         [(1, 3), (3, 5), (5, 7)])

    def test_unsorted_overlaps_merge_without_changing_input(self):
        intervals = [(6, 8), (3, 6), (9, 11), (1, 4), (2, 3)]
        original = intervals.copy()
        self.assertEqual(merge(intervals), [(1, 6), (6, 8), (9, 11)])
        self.assertEqual(intervals, original)

    def test_nested_and_duplicate_intervals_merge(self):
        self.assertEqual(merge([(2, 4), (1, 8), (1, 8), (3, 5)]),
                         [(1, 8)])

    def test_overlap_bridge_merges_touching_intervals(self):
        self.assertEqual(merge([(1, 3), (3, 5), (2, 4)]), [(1, 5)])

    def test_empty_and_single_interval(self):
        self.assertEqual(merge([]), [])
        self.assertEqual(merge([(-2, 1)]), [(-2, 1)])
