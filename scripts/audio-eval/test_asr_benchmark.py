import tempfile
import unittest
from pathlib import Path

from asr_benchmark import normalize, score, summarize, validate_cases, speech_normalized_score


class AsrBenchmarkTests(unittest.TestCase):
    def test_fixed_normalization_is_shared_and_language_specific(self):
        self.assertEqual(normalize("Hello, ＷＯＲＬＤ!", "en"), ["hello", "world"])
        self.assertEqual(normalize("hello,world", "en"), ["hello", "world"])
        self.assertEqual(normalize("你 好，ＡＩ！", "zh"), list("你好ai"))
        self.assertEqual(score("hello world", "hello brave world", "en")["errors"], 1)
        self.assertEqual(score("你好世界", "你好", "zh")["errors"], 2)

    def test_exact_scoring_does_not_fall_back_for_large_inputs(self):
        result = score("a" * 4000, "b" + "a" * 4000, "zh")
        self.assertEqual(result["errors"], 1)
        self.assertEqual(result["method"], "levenshtein")

    def test_speech_normalization_is_separate_from_literal_word_errors(self):
        self.assertGreater(score("one hundred percent", "100%", "en")["errors"], 0)
        self.assertEqual(speech_normalized_score("one hundred percent", "100%")["errors"], 0)

    def test_corpus_weighting_uses_reference_units_and_retains_failures(self):
        cases = [
            {"id": "a", "language": "en", "durationSeconds": 1, "reference": "one"},
            {
                "id": "b",
                "language": "en",
                "durationSeconds": 9,
                "reference": "one two three four five six seven eight nine",
            },
        ]
        rows = [
            {"caseId": "a", "status": "ok", "text": "bad", "wallSeconds": 1, "repeat": 0},
            {
                "caseId": "b",
                "status": "ok",
                "text": cases[1]["reference"],
                "wallSeconds": 1,
                "repeat": 0,
            },
        ]
        group = summarize(cases, rows)[0]
        self.assertAlmostEqual(group["errorRate"], 0.1)
        self.assertAlmostEqual(group["rtf"], 0.2)
        rows[1] = {
            "caseId": "b",
            "status": "failed",
            "error": "model failed",
            "wallSeconds": 2,
            "repeat": 0,
        }
        group = summarize(cases, rows)[0]
        self.assertEqual(group["failures"], 1)
        self.assertIsNone(group["errorRate"])
        with self.assertRaises(ValueError):
            summarize(cases, rows[:1])
        with self.assertRaises(ValueError):
            summarize(cases, [rows[0], rows[0]])

    def test_manifest_hash_mismatch_is_rejected_before_model_execution(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "audio.wav"
            path.write_bytes(b"not the pinned audio")
            with self.assertRaises(ValueError):
                validate_cases(
                    [
                        {
                            "id": "a",
                            "audioPath": str(path),
                            "sha256": "0" * 64,
                            "durationSeconds": 1,
                            "reference": "gold",
                            "language": "en",
                        }
                    ]
                )


if __name__ == "__main__":
    unittest.main()
