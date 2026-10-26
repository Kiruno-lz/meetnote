import unittest
import importlib.metadata
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from unittest import mock

from metrics import diarization, edit_distance, score_record, tokens


class MetricsTests(unittest.TestCase):
    def test_tokenization_and_edit_distance(self):
        self.assertEqual(tokens("你好 世界", "zh"), ["你", "好", "世", "界"])
        self.assertEqual(tokens("Hello, world!", "en"), ["hello", ",", "world", "!"])
        self.assertEqual(edit_distance(["a", "b"], ["a", "c"]), 1)


class MetricsAvailabilityTests(unittest.TestCase):
    def test_missing_standard_scorer_is_unavailable_instead_of_perfect_der(self):
        reference = {"meetingId": "m", "transcript": [], "referenceSpeakerSegments": []}
        prediction = {
            "schemaVersion": 2,
            "predictionKind": "audio_host_prediction",
            "sourceDomain": "audio",
            "assistantSemanticQualityStatus": "not_assessed",
            "transcript": [],
            "speakerSegments": [],
        }
        with mock.patch(
            "metrics.importlib.metadata.version",
            side_effect=importlib.metadata.PackageNotFoundError,
        ):
            result = score_record(reference, prediction)
        self.assertEqual(result["diarization"]["status"], "unavailable")
        self.assertIsNone(result["diarization"]["der"])

    def test_missing_diarization_is_distinct_from_explicit_no_speech(self):
        reference = {
            "meetingId": "m",
            "transcript": [],
            "referenceSpeakerSegments": [{"startMs": 0, "endMs": 1000, "speaker": "A"}],
        }
        prediction = {
            "schemaVersion": 2,
            "predictionKind": "audio_host_prediction",
            "sourceDomain": "audio",
            "assistantSemanticQualityStatus": "not_assessed",
        }
        result = score_record(reference, prediction)
        self.assertEqual(result["diarization"]["status"], "unavailable")
        self.assertIsNone(result["diarization"]["der"])
        self.assertIn("no speakerSegments", result["diarization"]["reason"])

    def test_invalid_intervals_are_rejected_before_optional_dependency_loading(self):
        from metrics import EvaluationError

        for start, end, speaker in [
            (1, 1, "A"),
            (-1, 2, "A"),
            (0, float("nan"), "A"),
            (False, 1000, "A"),
            (0, 1000, ""),
        ]:
            with self.subTest(start=start, end=end, speaker=speaker):
                with self.assertRaises(EvaluationError):
                    diarization([{"startMs": start, "endMs": end, "speaker": speaker}], [])
        for kwargs in [{"collar_ms": -1}, {"collar_ms": 10**400}, {"duration_ms": 10**400}]:
            with self.assertRaises(EvaluationError):
                diarization([], [], **kwargs)
        with self.assertRaises(EvaluationError):
            diarization([{"startMs": 0, "endMs": 2000, "speaker": "A"}], [], duration_ms=1000)


class MetricsCliTests(unittest.TestCase):
    def fixture(self, root):
        ref = {
            "meetingId": "m",
            "transcript": [],
            "referenceSpeakerSegments": [{"startMs": 0, "endMs": 1000, "speaker": "A"}],
        }
        pred = {
            "meetingId": "m",
            "schemaVersion": 2,
            "predictionKind": "audio_host_prediction",
            "sourceDomain": "audio",
            "assistantSemanticQualityStatus": "not_assessed",
            "transcript": [],
            "speakerSegments": [],
        }
        for name, row in [("reference", ref), ("prediction", pred)]:
            (root / f"{name}.jsonl").write_text(json.dumps(row) + "\n")
        return ref, pred

    def run_cli(self, root, output):
        return subprocess.run(
            [
                sys.executable,
                str(Path(__file__).with_name("metrics.py")),
                "--reference",
                str(root / "reference.jsonl"),
                "--predictions",
                str(root / "prediction.jsonl"),
                "--output",
                str(output),
            ],
            capture_output=True,
            text=True,
            timeout=30,
        )

    def test_invalid_rerun_removes_previous_success_without_modifying_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ref, pred = self.fixture(root)
            output = root / "report.json"
            output.write_text('{"status":"old_success"}')
            pred["speakerSegments"] = [{"startMs": 1000, "endMs": 0, "speaker": "X"}]
            source = root / "prediction.jsonl"
            source.write_text(json.dumps(pred) + "\n")
            before = source.read_bytes()
            result = self.run_cli(root, output)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(
                output.exists(), "invalid scoring must not leave an old successful report"
            )
            self.assertEqual(source.read_bytes(), before)

    def test_empty_duplicate_and_unexpected_meetings_are_rejected(self):
        for kind in ["empty", "duplicate", "unexpected"]:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                ref, pred = self.fixture(root)
                if kind == "empty":
                    (root / "reference.jsonl").write_text("")
                elif kind == "duplicate":
                    (root / "prediction.jsonl").write_text((json.dumps(pred) + "\n") * 2)
                else:
                    (root / "prediction.jsonl").write_text(
                        json.dumps(pred) + "\n" + json.dumps(pred | {"meetingId": "other"}) + "\n"
                    )
                result = self.run_cli(root, root / "report.json")
                self.assertNotEqual(result.returncode, 0, kind)
                self.assertFalse((root / "report.json").exists())

    def test_report_hash_matches_the_reference_bytes_actually_scored(self):
        import contextlib
        import hashlib
        import io
        import metrics

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            source = root / "reference.jsonl"
            before = source.read_bytes()
            output = root / "report.json"
            original = metrics.records_by_id

            def concurrent_edit(path, *args):
                result = original(path, *args)
                if path == source:
                    path.write_bytes(before + b"\n")
                return result

            with mock.patch.object(
                metrics, "records_by_id", side_effect=concurrent_edit
            ), mock.patch.object(
                sys,
                "argv",
                [
                    "metrics",
                    "--reference",
                    str(source),
                    "--predictions",
                    str(root / "prediction.jsonl"),
                    "--output",
                    str(output),
                ],
            ), contextlib.redirect_stdout(
                io.StringIO()
            ):
                metrics.main()
            self.assertEqual(
                json.loads(output.read_text())["referenceSha256"],
                hashlib.sha256(before).hexdigest(),
            )

    def test_missing_input_clears_an_old_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            (root / "reference.jsonl").unlink()
            output = root / "report.json"
            output.write_text('{"status":"old_success"}')
            result = self.run_cli(root, output)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())

    def test_inputs_cannot_be_report_outputs_even_through_a_hardlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            source = root / "reference.jsonl"
            before = source.read_bytes()
            for path in [source, root / "linked.jsonl"]:
                if path != source:
                    path.hardlink_to(source)
                result = self.run_cli(root, path)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(source.read_bytes(), before)


def has_standard_der():
    try:
        return importlib.metadata.version("pyannote.metrics") == "4.0.0"
    except importlib.metadata.PackageNotFoundError:
        return False


@unittest.skipUnless(has_standard_der(), "requires requirements-diarization.txt")
class DiarizationMetricsTests(unittest.TestCase):
    def test_perfect_asr_and_diarization_after_label_permutation(self):
        reference = {
            "meetingId": "m1",
            "language": "en",
            "transcript": [{"startMs": 0, "endMs": 1000, "speaker": "A", "text": "Hello"}],
            "referenceSpeakerSegments": [{"startMs": 0, "endMs": 1000, "speaker": "A"}],
        }
        prediction = {
            "schemaVersion": 2,
            "predictionKind": "audio_host_prediction",
            "sourceDomain": "audio",
            "assistantSemanticQualityStatus": "not_assessed",
            "meetingId": "m1",
            "transcript": [{"startMs": 0, "endMs": 1000, "speaker": "X", "text": "Hello"}],
            "speakerSegments": [{"startMs": 0, "endMs": 1000, "speaker": "X"}],
        }
        result = score_record(reference, prediction)
        self.assertEqual(result["asr"]["werOrCer"], 0)
        self.assertEqual(result["diarization"]["der"], 0)

    def test_speaker_substitution_counts_once_instead_of_as_miss_and_false_alarm(self):
        reference = [
            {"startMs": 0, "endMs": 1000, "speaker": "A"},
            {"startMs": 1000, "endMs": 2000, "speaker": "B"},
        ]
        prediction = [{"startMs": 0, "endMs": 2000, "speaker": "X"}]
        result = diarization(reference, prediction)
        self.assertEqual(result["der"], 0.5)
        self.assertEqual(result["missRate"], 0)
        self.assertEqual(result["falseAlarmRate"], 0)
        self.assertEqual(result["confusionRate"], 0.5)

    def test_optimal_mapping_considers_extra_hypothesis_labels_and_is_rename_invariant(self):
        reference = [{"startMs": 0, "endMs": 1000, "speaker": "A"}]
        for first, second in [("X", "Y"), ("Y", "X")]:
            result = diarization(
                reference,
                [
                    {"startMs": 0, "endMs": 10, "speaker": first},
                    {"startMs": 10, "endMs": 1000, "speaker": second},
                ],
            )
            self.assertAlmostEqual(result["der"], 0.01)
            self.assertEqual(result["missRate"], 0)
            self.assertEqual(result["falseAlarmRate"], 0)

    def test_overlapping_intervals_of_the_same_speaker_keep_speech_active(self):
        reference = [
            {"startMs": 0, "endMs": 1000, "speaker": "A"},
            {"startMs": 500, "endMs": 1500, "speaker": "A"},
        ]
        result = diarization(reference, [{"startMs": 0, "endMs": 1500, "speaker": "X"}])
        self.assertEqual(result["der"], 0)

    def test_reference_collars_exclude_boundaries_instead_of_expanding_speech(self):
        reference = [{"startMs": 1000, "endMs": 2000, "speaker": "A"}]
        prediction = [{"startMs": 1100, "endMs": 2100, "speaker": "X"}]
        self.assertEqual(
            diarization(reference, prediction, collar_ms=200, duration_ms=3000)["der"], 0
        )
        prediction[0]["endMs"] = 2500
        result = diarization(reference, prediction, collar_ms=200, duration_ms=3000)
        self.assertAlmostEqual(result["scoredSpeakerSeconds"], 0.8)
        self.assertAlmostEqual(result["falseAlarmSpeakerSeconds"], 0.4)
        self.assertAlmostEqual(result["der"], 0.5)

    def test_overlap_is_scored_in_speaker_time_and_missing_speech_is_not_confusion(self):
        reference = [{"startMs": 0, "endMs": 1000, "speaker": s} for s in ["A", "B"]]
        result = diarization(reference, [{"startMs": 0, "endMs": 1000, "speaker": "X"}])
        self.assertEqual(result["scoredSpeakerSeconds"], 2)
        self.assertEqual(result["der"], 0.5)
        self.assertEqual(result["missRate"], 0.5)
        self.assertEqual(result["confusionRate"], 0)

    def test_empty_or_fully_excluded_reference_has_no_numeric_der(self):
        for reference, prediction, collar in [
            ([], [], 0),
            ([], [{"startMs": 0, "endMs": 1000, "speaker": "X"}], 0),
            ([{"startMs": 0, "endMs": 100, "speaker": "A"}], [], 200),
        ]:
            result = diarization(reference, prediction, collar_ms=collar)
            self.assertEqual(result["status"], "unscored_no_reference_speech")
            self.assertIsNone(result["der"])
        result = diarization([{"startMs": 0, "endMs": 1000, "speaker": "A"}], [])
        self.assertEqual(result["der"], 1)
        self.assertEqual(result["missRate"], 1)

    def test_many_speakers_use_optimal_assignment_without_factorial_enumeration(self):
        reference = [
            {"startMs": i * 1000, "endMs": (i + 1) * 1000, "speaker": str(i)} for i in range(20)
        ]
        prediction = [dict(s, speaker=str(19 - int(s["speaker"]))) for s in reference]
        self.assertEqual(diarization(reference, prediction)["der"], 0)

    def test_diarization_counts_false_alarm(self):
        reference = [{"startMs": 0, "endMs": 1000, "speaker": "A"}]
        prediction = [
            {"startMs": 0, "endMs": 1000, "speaker": "X"},
            {"startMs": 1000, "endMs": 1500, "speaker": "X"},
        ]
        result = diarization(reference, prediction)
        self.assertGreater(result["falseAlarmRate"], 0)
        self.assertGreater(result["der"], 0)


if __name__ == "__main__":
    unittest.main()
