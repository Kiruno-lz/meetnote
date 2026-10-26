import tempfile
import unittest
from pathlib import Path

from asr_benchmark import (
    normalize,
    score,
    summarize,
    validate_cases,
    speech_normalized_score,
    merged_intervals,
)


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

    def test_known_silence_is_measured_as_interval_union_without_claiming_alignment(self):
        case = {
            "id": "pause",
            "language": "zh",
            "durationSeconds": 5,
            "reference": "你好",
            "knownSilenceRangesMs": [[1000, 3000], [2000, 4000]],
        }
        row = {
            "caseId": "pause",
            "repeat": 0,
            "status": "ok",
            "wallSeconds": 0.1,
            "transcript": [
                {"startMs": 0, "endMs": 2000, "text": "你"},
                {"startMs": 1500, "endMs": 3500, "text": "好"},
            ],
        }
        timing = summarize([case], [row])[0]["knownSilence"]
        self.assertEqual(timing["scope"], "known_silence_only_not_word_alignment")
        self.assertEqual(timing["referenceMs"], 3000)
        self.assertEqual(timing["overlapMs"], 2500)
        self.assertEqual(timing["measuredCases"], 1)
        failed = dict(row, status="failed")
        timing = summarize([case], [failed])[0]["knownSilence"]
        self.assertIsNone(timing["overlapMs"], "failure is not zero silence overlap")
        self.assertEqual(timing["measuredCases"], 0)

    def test_timing_intervals_reject_nonfinite_boolean_and_out_of_range_values(self):
        for spans in (
            [[0, float("nan")]],
            [[False, 100]],
            [[-1, 100]],
            [[0, 6000]],
            [[4, 4]],
            [[1]],
        ):
            with self.subTest(spans=spans), self.assertRaises(ValueError):
                merged_intervals(spans, 5000)
        self.assertEqual(
            merged_intervals([[2000, 3000], [0, 1000], [1000, 2500]], 5000), [(0, 3000)]
        )

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
