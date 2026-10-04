import json
import tempfile
import unittest
from pathlib import Path

from diagnose_ami import diagnose


class AmiDiagnosisTests(unittest.TestCase):
    def test_separates_input_integrity_from_speaker_boundary_quality(self):
        record = {
            "scenarioId": "ami-test",
            "segments": [
                {"id": "a", "startMs": 0, "endMs": 100, "speaker": "A", "text": "topic one"},
                {"id": "b", "startMs": 90, "endMs": 200, "speaker": "B", "text": "topic two"},
            ],
            "gold": {
                "allowed_mode": "partition",
                "children": [
                    {
                        "start_segment_id": "a",
                        "end_segment_id": "a",
                        "evidence_segment_ids": ["a"],
                    },
                    {
                        "start_segment_id": "b",
                        "end_segment_id": "b",
                        "evidence_segment_ids": ["b"],
                    },
                ],
                "uncovered_ranges": [],
            },
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "ami.jsonl"
            path.write_text(json.dumps(record) + "\n", encoding="utf-8")
            report = diagnose(path)
        self.assertEqual(report["inputIntegrity"]["status"], "passed")
        self.assertEqual(report["timing"]["overlappingAdjacentPairs"], 1)
        self.assertEqual(report["speakerChangeVsTopicBoundary"]["boundaryF1"], 1.0)
        self.assertEqual(report["asrQualityStatus"], "not_assessed")


if __name__ == "__main__":
    unittest.main()
