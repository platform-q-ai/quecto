import unittest

from classification import classify_name
from inventory import summarize


class ClassificationParity(unittest.TestCase):
    def test_filename_rules(self):
        cases = [
            ("a.txt", "text"),
            ("a.md", "text"),
            ("a.csv", "data"),
            ("a.json", "data"),
            ("a.TXT", "other"),
            ("a.MD", "other"),
            ("a.CSV", "other"),
            ("a.JSON", "other"),
            ("a.Txt", "other"),
            ("a.txt.csv", "data"),
            ("a.json.md", "text"),
            ("a.txt.gz", "other"),
            ("a.csv.", "other"),
            (".txt", "text"),
            (".json", "data"),
            (".", "other"),
            ("", "other"),
            ("txt", "other"),
            ("README", "other"),
            ("a.xml", "other"),
            ("a.yaml", "other"),
            ("a.py", "other"),
            ("a.txt ", "other"),
        ]
        for name, category in cases:
            with self.subTest(name=name):
                self.assertEqual(classify_name(name), category)
                self.assertEqual(summarize([name]), {category: 1})

        self.assertEqual(
            list(summarize(name for name, _ in cases).items()),
            [("text", 4), ("data", 4), ("other", 15)],
        )

    def test_category_order_and_repeated_names(self):
        cases = [
            (["a.bin", "a.csv", "a.txt", "a.csv"],
             [("other", 1), ("data", 2), ("text", 1)]),
            (["a.json", "a.md", "a.bin", "a.md"],
             [("data", 1), ("text", 2), ("other", 1)]),
        ]
        for names, expected in cases:
            with self.subTest(names=names):
                self.assertEqual(list(summarize(iter(names)).items()), expected)

    def test_empty(self):
        self.assertEqual(summarize([]), {})
        self.assertEqual(summarize(iter(())), {})
