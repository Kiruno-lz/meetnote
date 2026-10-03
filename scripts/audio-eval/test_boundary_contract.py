import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "assistant-eval"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from evaluate_composite import EvaluationError, evaluate  # noqa: E402
from compose_semantic_audio import reference_payload  # noqa: E402
from metrics import EvaluationError as MetricsEvaluationError  # noqa: E402
from metrics import score_record  # noqa: E402


def reference():
    return {
        "schemaVersion": 2,
        "type": "compositeAudioBoundaryReference",
        "evaluationKind": "audio_boundary_reference",
        "sourceDomain": "audio",
        "durationMs": 2000,
        "segments": [
            {"startMs": 0, "endMs": 1000, "topic": "one"},
            {"startMs": 1000, "endMs": 2000, "topic": "two"},
        ],
        "assistantSemanticQualityStatus": "not_assessed",
    }


def audio_prediction():
    return {
        "schemaVersion": 2,
        "predictionKind": "audio_host_prediction",
        "sourceDomain": "audio",
        "durationMs": 2000,
        "segments": [
            {"startMs": 0, "endMs": 980},
            {"startMs": 980, "endMs": 2000},
        ],
        "assistantSemanticQualityStatus": "not_assessed",
    }


class BoundaryContractTests(unittest.TestCase):
    def test_composer_emits_audio_only_reference_with_not_assessed_semantics(self):
        payload = reference_payload(
            2000,
            [
                {"startMs": 0, "endMs": 1000, "topic": "fixture-only"},
                {"startMs": 1000, "endMs": 2000, "topic": "fixture-only"},
            ],
        )
        self.assertEqual(payload["evaluationKind"], "audio_boundary_reference")
        self.assertEqual(payload["assistantSemanticQualityStatus"], "not_assessed")
        self.assertNotIn("assistantSemanticLabels", payload)

    def test_audio_boundary_report_is_not_assistant_quality(self):
        report = evaluate(reference(), audio_prediction())
        self.assertEqual(report["status"], "audio_boundary_scored")
        self.assertEqual(report["qualityStatus"], "not_assessed")
        self.assertEqual(report["evaluationKind"], "audio_boundary_smoke")
        self.assertNotIn("assistantSemanticScore", report)

    def test_assistant_semantic_prediction_is_rejected(self):
        prediction = audio_prediction()
        prediction["predictionKind"] = "assistant_semantic_prediction"
        with self.assertRaisesRegex(EvaluationError, "Assistant semantic"):
            evaluate(reference(), prediction)

    def test_legacy_untyped_prediction_is_rejected(self):
        prediction = {"segments": audio_prediction()["segments"]}
        with self.assertRaisesRegex(EvaluationError, "schemaVersion"):
            evaluate(reference(), prediction)

    def test_reference_cannot_claim_assistant_semantic_quality(self):
        fixture = reference()
        fixture["assistantSemanticQualityStatus"] = "passed"
        with self.assertRaisesRegex(EvaluationError, "not_assessed"):
            evaluate(fixture, audio_prediction())

    def test_audio_metrics_reject_assistant_prediction_and_declare_boundary(self):
        reference_record = {
            "meetingId": "audio-fixture",
            "language": "en",
            "transcript": [
                {"startMs": 0, "endMs": 1000, "speaker": "A", "text": "hello"}
            ],
            "referenceSpeakerSegments": [
                {"startMs": 0, "endMs": 1000, "speaker": "A"}
            ],
        }
        prediction = {
            "schemaVersion": 2,
            "predictionKind": "audio_host_prediction",
            "sourceDomain": "audio",
            "assistantSemanticQualityStatus": "not_assessed",
            "transcript": [
                {"startMs": 0, "endMs": 1000, "speaker": "X", "text": "hello"}
            ],
            "speakerSegments": [
                {"startMs": 0, "endMs": 1000, "speaker": "X"}
            ],
        }
        result = score_record(reference_record, prediction)
        self.assertEqual(result["evaluationKind"], "audio_host_quality")
        self.assertEqual(result["assistantSemanticQualityStatus"], "not_assessed")

        prediction["predictionKind"] = "assistant_semantic_prediction"
        with self.assertRaisesRegex(MetricsEvaluationError, "Assistant semantic"):
            score_record(reference_record, prediction)

    def test_audio_metrics_reject_gold_speaker_segments_in_prediction(self):
        reference_record = {
            "meetingId": "audio-fixture",
            "language": "en",
            "transcript": [],
            "referenceSpeakerSegments": [
                {"startMs": 0, "endMs": 1000, "speaker": "A"}
            ],
        }
        prediction = audio_prediction() | {
            "referenceSpeakerSegments": [
                {"startMs": 0, "endMs": 1000, "speaker": "A"}
            ]
        }
        with self.assertRaisesRegex(MetricsEvaluationError, "gold annotations"):
            score_record(reference_record, prediction)


if __name__ == "__main__":
    unittest.main()
