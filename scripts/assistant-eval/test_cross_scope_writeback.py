#!/usr/bin/env python3

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from cross_scope_eval import _normalize_prediction, load_json
from cross_scope_writeback import (
    _public_document_record,
    _submission_request,
    _writeback_children,
    run_submission,
)


ROOT = Path(__file__).resolve().parent


class CrossScopeWritebackTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = load_json(ROOT / "fixtures" / "cross-scope.json")
        cls.case = next(item for item in cls.fixture["cases"] if item["caseId"] == "cross-block-partition")

    def test_submission_request_and_document_are_public_only(self):
        request = _submission_request(self.fixture, self.case, "openaiResponses")
        encoded = json.dumps(request, ensure_ascii=False)
        self.assertNotIn('"gold"', encoded)
        self.assertNotIn('"negativeCases"', encoded)
        self.assertNotIn('"semanticRationale"', encoded)
        record = _public_document_record(self.fixture, self.case)
        self.assertNotIn("gold", record)
        self.assertNotIn("negativeCase", record)
        self.assertEqual(len(record["record"]["state"]["results"]["transcript"]), 4)

    def test_compact_public_prediction_converts_to_cli_ranges(self):
        prediction = {
            "scenarioId": self.case["caseId"],
            "mode": "partition",
            "children": [
                {
                    "childId": "one",
                    "documentId": "doc-alpha",
                    "blockId": "doc-alpha:block-1",
                    "segmentIds": ["alpha-1", "alpha-2"],
                    "evidenceSegmentIds": ["alpha-1"],
                    "title": "Release planning",
                    "summary": "Release planning evidence.",
                    "tags": ["release"],
                },
                {
                    "childId": "two",
                    "documentId": "doc-alpha",
                    "blockId": "doc-alpha:block-2",
                    "segmentIds": ["alpha-3", "alpha-4"],
                    "evidenceSegmentIds": ["alpha-3"],
                    "title": "Budget decision",
                    "summary": "Budget decision evidence.",
                    "tags": ["budget"],
                },
            ],
            "uncoveredSpans": [],
            "uncovered_ranges": [],
        }
        normalized = _normalize_prediction(self.fixture, self.case, prediction)
        children = _writeback_children(
            normalized,
            ["alpha-1", "alpha-2", "alpha-3", "alpha-4"],
            {},
        )
        self.assertEqual(children[0]["start_segment_id"], "alpha-1")
        self.assertEqual(children[1]["end_segment_id"], "alpha-4")
        self.assertNotIn("mutationId", children[0])

    def test_not_assessed_runner_writes_structured_report_without_receipt(self):
        def fake_runner(*_args, **_kwargs):
            from cross_scope_eval import CrossScopeError

            raise CrossScopeError("NOT_ASSESSED", "missing external model", status="not_assessed")

        with tempfile.TemporaryDirectory() as temporary:
            with patch("cross_scope_writeback._run_external", side_effect=fake_runner):
                report = run_submission(
                    self.fixture,
                    runner="fake-runner",
                    protocol="openaiResponses",
                    cli="fake-cli",
                    package_root=Path(temporary),
                )
        self.assertEqual(report["status"], "not_assessed")
        self.assertNotIn("mutationReceipts", report)
        self.assertNotIn("readback", report)


if __name__ == "__main__":
    unittest.main()
