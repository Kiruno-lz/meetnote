#!/usr/bin/env python3

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from evaluate import (
    boundary_prf,
    equal_length_labels,
    pk,
    scores,
    select_split,
    validate_run_metadata,
    validate_gold_record,
    validate_structure,
    window_diff,
)


class MetricTests(unittest.TestCase):
    def test_run_metadata_requires_auditable_fields_and_rejects_secrets(self):
        metadata = {
            "model": "fixture-model",
            "prompt": "topic partition v1",
            "budget": {"maxToolRounds": 8},
            "elapsedMs": 120,
            "resources": {"peakMemoryMb": 64},
        }
        self.assertEqual(validate_run_metadata(metadata), metadata)
        with self.assertRaises(ValueError):
            validate_run_metadata({**metadata, "apiKey": "must-not-be-recorded"})
        with self.assertRaisesRegex(ValueError, "forbidden secret"):
            validate_run_metadata({**metadata, "resources": {"nested": {"token": "must-not-be-recorded"}}})
        with self.assertRaisesRegex(ValueError, "elapsedMs"):
            validate_run_metadata({**metadata, "elapsedMs": -1})
        with self.assertRaises(ValueError):
            validate_run_metadata({"model": "fixture-model"})

    def test_fixed_smoke_and_full_splits_are_disjoint(self):
        records = [
            {"scenarioId": "smoke", "smoke": True},
            {"scenarioId": "full", "smoke": False},
        ]
        self.assertEqual([item["scenarioId"] for item in select_split(records, "smoke")], ["smoke"])
        self.assertEqual([item["scenarioId"] for item in select_split(records, "full")], ["full"])
        self.assertEqual(len(select_split(records, "all")), 2)

    def test_perfect_segmentation(self):
        result = scores([0, 0, 1, 1, 2, 2], [0, 0, 1, 1, 2, 2])
        self.assertEqual(result.boundary_f1, 1.0)
        self.assertEqual(result.pk, 0.0)
        self.assertEqual(result.window_diff, 0.0)

    def test_no_split_misses_reference_boundaries(self):
        reference = [0, 0, 1, 1, 2, 2]
        hypothesis = [0] * len(reference)
        precision, recall, f1 = boundary_prf(reference, hypothesis)
        self.assertEqual((precision, recall, f1), (0.0, 0.0, 0.0))
        self.assertGreater(pk(reference, hypothesis), 0.0)
        self.assertGreater(window_diff(reference, hypothesis), 0.0)

    def test_equal_length_is_deterministic(self):
        self.assertEqual(equal_length_labels(8, 3), [0, 0, 0, 1, 1, 1, 2, 2])

    def test_single_item_has_zero_error(self):
        self.assertEqual(pk([0], [5]), 0.0)
        self.assertEqual(window_diff([0], [5]), 0.0)

    def test_length_mismatch_is_rejected(self):
        with self.assertRaises(ValueError):
            pk([0, 1], [0])
        with self.assertRaises(ValueError):
            window_diff([0, 1], [0])

    def test_partition_rejects_gap_and_out_of_range_evidence(self):
        validation = validate_structure(
            ["s0", "s1", "s2"],
            "partition",
            [
                {
                    "start_segment_id": "s0",
                    "end_segment_id": "s0",
                    "evidence_segment_ids": ["s2"],
                },
                {
                    "start_segment_id": "s2",
                    "end_segment_id": "s2",
                    "evidence_segment_ids": ["s2"],
                },
            ],
            [],
        )
        self.assertFalse(validation.valid)
        self.assertFalse(validation.evidence_valid)
        self.assertEqual(validation.coverage, 2 / 3)

    def test_extract_requires_exact_gap_report(self):
        validation = validate_structure(
            ["s0", "s1", "s2"],
            "extract",
            [
                {
                    "start_segment_id": "s1",
                    "end_segment_id": "s1",
                    "evidence_segment_ids": ["s1"],
                }
            ],
            [{"start_segment_id": "s0", "end_segment_id": "s0"}],
        )
        self.assertFalse(validation.valid)
        self.assertFalse(validation.gap_report_valid)

    def test_public_timing_allows_overlapping_speakers_but_not_reverse_order(self):
        record = {
            "scenarioId": "overlap",
            "segments": [
                {"id": "s0", "startMs": 0, "endMs": 2000},
                {"id": "s1", "startMs": 1000, "endMs": 3000},
            ],
            "gold": {
                "allowed_mode": "partition",
                "children": [
                    {
                        "start_segment_id": "s0",
                        "end_segment_id": "s1",
                        "title": ["topic"],
                        "tags": [["topic"]],
                        "summary": [],
                        "evidence_segment_ids": ["s0", "s1"],
                    }
                ],
                "uncovered_ranges": [],
            },
        }
        self.assertTrue(validate_gold_record(record).valid)
        record["segments"][1]["startMs"] = -1
        self.assertFalse(validate_gold_record(record).valid)


if __name__ == "__main__":
    unittest.main()
