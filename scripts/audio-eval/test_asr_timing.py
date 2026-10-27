import hashlib
import copy
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


@unittest.skipUnless(importlib.util.find_spec("meeteval"), "requires requirements-timing.txt")
class AsrTimingTests(unittest.TestCase):
    def fixture(self, root, status="ok"):
        audio = root / "fixture.wav"
        audio.write_bytes(b"protocol fixture; no model inference")
        case = {
            "id": "fixture",
            "audioPath": str(audio),
            "sha256": hashlib.sha256(audio.read_bytes()).hexdigest(),
            "language": "en",
            "scenario": "timing-fixture",
            "durationSeconds": 60,
            "reference": "hello world",
            "wordTiming": {
                "source": "test-reference",
                "manuallyVerified": False,
                "sampleRateHz": 16000,
                "words": [
                    {"id": "a", "text": "hello", "startSample": 0, "endSample": 16000},
                    {"id": "b", "text": "world", "startSample": 16000, "endSample": 32000},
                ],
            },
        }
        row = {
            "caseId": "fixture",
            "repeat": 0,
            "status": status,
            "wallSeconds": 0.1,
            "transcript": [
                {"text": "hello", "startMs": 20000, "endMs": 21000},
                {"text": "world", "startMs": 21000, "endMs": 22000},
            ],
        }
        case_bytes = (json.dumps([case]) + "\n").encode()
        prediction = (json.dumps(row) + "\n").encode()
        (root / "cases.json").write_bytes(case_bytes)
        (root / "prediction.jsonl").write_bytes(prediction)
        (root / "config.json").write_text(json.dumps({"repeats": 1}))
        report = {
            "failure": None,
            "config": {"repeats": 1},
            "casesSha256": hashlib.sha256(case_bytes).hexdigest(),
            "predictionSha256": hashlib.sha256(prediction).hexdigest(),
        }
        (root / "report.json").write_text(json.dumps(report))

    def run_cli(self, root, output, collars=("2",)):
        return subprocess.run(
            [
                sys.executable,
                str(Path(__file__).with_name("report_asr_timing.py")),
                "--run",
                str(root),
                "--output",
                str(output),
                "--collar",
                *collars,
            ],
            capture_output=True,
            text=True,
            timeout=30,
        )

    def test_correct_text_with_shifted_timing_is_not_reported_as_temporally_correct(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            output = root / "timing.json"
            result = self.run_cli(root, output)
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads(output.read_text())
            self.assertEqual(report["qualityAcceptance"], "not_assessed")
            group = report["groups"][0]
            self.assertEqual(group["literalErrorRate"], 0)
            self.assertEqual(group["timeConstrainedErrorRate"], 2)

    def test_aligned_words_and_a_whole_file_caption_are_distinguished(self):
        from report_asr_timing import temporal_score

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            case = json.loads((root / "cases.json").read_text())[0]
            row = json.loads((root / "prediction.jsonl").read_text())
            for segment in row["transcript"]:
                segment["startMs"] -= 20000
                segment["endMs"] -= 20000
            self.assertEqual(temporal_score(case, row, 2)["timeConstrainedErrors"], 0)
            row["transcript"] = [{"text": "hello world", "startMs": 0, "endMs": 60000}]
            result = temporal_score(case, row, 2)
            self.assertEqual(result["literalErrors"], 0)
            self.assertEqual(result["timeConstrainedErrors"], 4)

    def test_failed_recheck_removes_previous_complete_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            output = root / "timing.json"
            self.assertEqual(self.run_cli(root, output).returncode, 0)
            (root / "prediction.jsonl").write_text("changed")
            self.assertNotEqual(self.run_cli(root, output).returncode, 0)
            self.assertFalse(output.exists(), "stale temporal receipts must not survive failures")

    def test_output_cannot_replace_input_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            for name in ["report.json", "prediction.jsonl", "cases.json", "config.json"]:
                output = root / name
                before = output.read_bytes()
                self.assertNotEqual(self.run_cli(root, output).returncode, 0)
                self.assertEqual(output.read_bytes(), before)

    def test_failed_inference_is_unmeasured_instead_of_zero_temporal_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root, status="failed")
            output = root / "timing.json"
            self.assertEqual(self.run_cli(root, output).returncode, 2)
            report = json.loads(output.read_text())
            self.assertEqual(report["status"], "incomplete")
            self.assertEqual(report["groups"][0]["failures"], 1)
            self.assertIsNone(report["groups"][0]["timeConstrainedErrorRate"])

    def test_invalid_or_inconsistent_timing_is_rejected_without_guessing(self):
        from report_asr_timing import temporal_score

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            original_case = json.loads((root / "cases.json").read_text())[0]
            original_row = json.loads((root / "prediction.jsonl").read_text())
            for failure in [
                "reference-text",
                "reference-range",
                "reference-order",
                "missing-timing",
                "nan",
                "boolean",
                "prediction-text",
            ]:
                case, row = copy.deepcopy(original_case), copy.deepcopy(original_row)
                if failure == "reference-text":
                    case["wordTiming"]["words"][0]["text"] = "other"
                elif failure == "reference-range":
                    case["wordTiming"]["words"][0]["startSample"] = -1
                elif failure == "reference-order":
                    case["wordTiming"]["words"].reverse()
                elif failure == "missing-timing":
                    del row["transcript"]
                elif failure == "nan":
                    row["transcript"][0]["startMs"] = float("nan")
                elif failure == "boolean":
                    row["transcript"][0]["endMs"] = True
                else:
                    row["text"] = "other"
                with self.subTest(failure=failure), self.assertRaises(ValueError):
                    temporal_score(case, row, 2)

    def test_duplicate_collars_do_not_duplicate_case_counts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            output = root / "timing.json"
            self.assertNotEqual(self.run_cli(root, output, ("2", "2")).returncode, 0)
            self.assertFalse(output.exists())

    def test_integer_milliseconds_at_eof_are_not_rejected_by_float_duration_roundoff(self):
        from report_asr_timing import temporal_score

        case = {
            "language": "en",
            "durationSeconds": 1.001,
            "reference": "hello",
            "wordTiming": {
                "source": "fixture",
                "manuallyVerified": False,
                "sampleRateHz": 16000,
                "words": [{"id": "w", "text": "hello", "startSample": 0, "endSample": 16016}],
            },
        }
        row = {"transcript": [{"text": "hello", "startMs": 0, "endMs": 1001}]}
        self.assertEqual(temporal_score(case, row, 2)["timeConstrainedErrors"], 0)


if __name__ == "__main__":
    unittest.main()
