import unittest

from settings import resolve


class ResolveTests(unittest.TestCase):
    def test_falsy_overrides_are_preserved(self):
        for override in (False, 0, ""):
            with self.subTest(override=override):
                result = resolve({"key": "default"}, {"key": override})
                self.assertEqual(result, {"key": override})
                self.assertIs(result["key"], override)

    def test_none_and_missing_keys_use_defaults(self):
        self.assertEqual(
            resolve({"present": 10, "missing": 20}, {"present": None}),
            {"present": 10, "missing": 20},
        )

    def test_new_dictionary_inputs_unchanged_and_unknown_keys_ignored(self):
        defaults = {"enabled": True, "port": 8000}
        overrides = {"enabled": False, "unknown": "ignored"}
        result = resolve(defaults, overrides)

        self.assertEqual(result, {"enabled": False, "port": 8000})
        self.assertIsNot(result, defaults)
        self.assertIsNot(result, overrides)
        self.assertEqual(defaults, {"enabled": True, "port": 8000})
        self.assertEqual(overrides, {"enabled": False, "unknown": "ignored"})

    def test_empty_defaults_return_new_empty_dictionary(self):
        defaults = {}
        overrides = {"unknown": 1}
        result = resolve(defaults, overrides)
        self.assertEqual(result, {})
        self.assertIsNot(result, defaults)
        self.assertIsNot(result, overrides)
