#!/usr/bin/env python3

import copy
import hashlib
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from semantic_schema import (
    SemanticSchemaError,
    validate_evidence_chain,
    validate_semantic_fixture,
    validate_semantic_prediction,
)


FIXTURE = Path(__file__).with_name("fixtures") / "cross-scope.json"


class SemanticSchemaTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_fixture_covers_cross_block_cross_document_all_modes_and_second_pass(self):
        summary = validate_semantic_fixture(self.fixture)
        self.assertEqual(summary["documentCount"], 2)
        self.assertEqual(summary["blockCount"], 4)
        self.assertEqual(summary["caseCount"], 4)
        self.assertEqual(summary["modes"], ["extract", "none", "partition"])

    def test_fixture_rejects_duplicate_or_omitted_segment_in_gold(self):
        duplicate = copy.deepcopy(self.fixture)
        children = duplicate["cases"][0]["gold"]["children"]
        children[1]["spans"][0]["segmentIds"].append("alpha-2")
        with self.assertRaisesRegex(SemanticSchemaError, "crosses block ownership|duplicate or overlapping"):
            validate_semantic_fixture(duplicate)

        omission = copy.deepcopy(self.fixture)
        omission["cases"][1]["gold"]["uncoveredSpans"][0]["segmentIds"].pop()
        with self.assertRaisesRegex(SemanticSchemaError, "extract has an omission"):
            validate_semantic_fixture(omission)

    def test_external_prediction_is_checked_against_document_and_block_scope(self):
        case = self.fixture["cases"][1]
        prediction = {"caseId": case["caseId"], **copy.deepcopy(case["gold"])}
        validate_semantic_prediction(self.fixture, prediction)
        prediction["children"][0]["spans"][1]["segmentIds"] = ["alpha-3"]
        with self.assertRaisesRegex(SemanticSchemaError, "wrong document ownership|crosses block ownership"):
            validate_semantic_prediction(self.fixture, prediction)

    def test_none_prediction_requires_evidence_based_rationale(self):
        case = self.fixture["cases"][2]
        prediction = {"caseId": case["caseId"], **copy.deepcopy(case["gold"])}
        prediction["semanticRationale"] = copy.deepcopy(case["semanticRationale"])
        validate_semantic_prediction(self.fixture, prediction)
        prediction.pop("semanticRationale")
        with self.assertRaisesRegex(SemanticSchemaError, "semanticRationale is required"):
            validate_semantic_prediction(self.fixture, prediction)

    def test_evidence_chain_requires_external_validator_correction_and_matching_readback(self):
        prediction_bytes = b'{"caseId":"validator_correction","correction":{"errorCode":"RANGE_OVERLAP","correctedOperation":{},"retry":false}}\n'
        digest = hashlib.sha256(prediction_bytes).hexdigest()
        receipts = [{"receiptId": "receipt-1", "operation": "update_document", "status": "committed"}]
        evidence = {
            "schemaVersion": 1,
            "metadata": {
                "runId": "run-1",
                "caseId": "validator_correction",
                "model": "external-test-model",
                "predictionSource": "external_runner",
                "promptSha256": "a" * 64,
                "resources": {"maxToolRounds": 8},
                "elapsedMs": 12,
            },
            "prediction": {
                "caseId": "validator_correction",
                "correction": {"errorCode": "RANGE_OVERLAP", "correctedOperation": {}, "retry": False},
            },
            "receipt": {"runId": "run-1", "predictionSha256": digest, "mutationReceipts": receipts},
            "readback": {"runId": "run-1", "predictionSha256": digest, "mutationReceipts": receipts},
        }
        validate_evidence_chain(evidence, expected_case_id="validator_correction", prediction_bytes=prediction_bytes)

        invalid = copy.deepcopy(evidence)
        invalid["prediction"]["correction"]["retry"] = True
        with self.assertRaisesRegex(SemanticSchemaError, "must not retry"):
            validate_evidence_chain(invalid, expected_case_id="validator_correction", prediction_bytes=prediction_bytes)


if __name__ == "__main__":
    unittest.main()
