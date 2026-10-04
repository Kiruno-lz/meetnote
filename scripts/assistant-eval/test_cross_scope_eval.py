#!/usr/bin/env python3

import copy
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from cross_scope_eval import (
    CrossScopeError,
    build_request,
    evaluate_case,
    load_json,
    run_evaluation,
)


ROOT = Path(__file__).resolve().parent
FIXTURE_PATH = ROOT / "fixtures" / "cross-scope.json"


def _public_prediction(case):
    gold = copy.deepcopy(case["gold"])
    prediction = {"scenarioId": case["caseId"], **gold, "uncovered_ranges": copy.deepcopy(gold.get("uncoveredSpans", gold.get("uncovered_ranges", [])))}
    if "uncovered_ranges" not in prediction:
        prediction["uncovered_ranges"] = []
    if case["family"] == "none":
        # This is model-produced evidence for the test double, not fixture
        # rationale.  The real request deliberately omits semanticRationale.
        prediction["semanticRationale"] = {
            "topicSupport": "insufficient",
            "evidenceSegmentIds": ["beta-3"],
        }
    if "secondPass" in prediction:
        prediction["second_pass"] = copy.deepcopy(prediction["secondPass"])
    return prediction


class CrossScopeEvalTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = load_json(FIXTURE_PATH)

    def test_request_contains_only_public_scope(self):
        case = self.fixture["cases"][0]
        request = build_request(self.fixture, case, "openaiResponses")
        encoded = json.dumps(request, ensure_ascii=False)
        self.assertNotIn('"gold"', encoded)
        self.assertNotIn('"negativeCases"', encoded)
        self.assertNotIn('"semanticRationale"', encoded)
        public_case = request["semanticFixtures"][0]["fixture"]["cases"][0]
        self.assertEqual(public_case["caseId"], case["caseId"])
        self.assertEqual(request["holdout"]["goldIncluded"], False)

    def test_none_request_requires_an_empty_operation(self):
        case = next(item for item in self.fixture["cases"] if item["family"] == "none")
        request = build_request(self.fixture, case, "openaiResponses")
        task = request["prompt"]["task"]
        self.assertIn('mode exactly to "none"', task)
        self.assertIn("children exactly as []", task)
        self.assertIn("uncoveredSpans exactly as []", task)
        self.assertIn("uncovered_ranges exactly as []", task)

    def test_nested_request_declares_public_selected_parent_scope(self):
        case = next(item for item in self.fixture["cases"] if item["family"] == "nested")
        request = build_request(self.fixture, case, "openaiResponses")
        task = request["prompt"]["task"]
        public_case = request["semanticFixtures"][0]["fixture"]["cases"][0]
        self.assertEqual(
            public_case["selectedParentSegmentIds"],
            ["alpha-1", "alpha-2", "alpha-3", "alpha-4"],
        )
        self.assertIn("exactly one top-level child", task)
        self.assertIn("secondPass.parentChildId", task)
        self.assertNotIn('"gold"', json.dumps(public_case, ensure_ascii=False))

    def test_independent_prediction_is_structurally_and_boundary_evaluated(self):
        case = next(item for item in self.fixture["cases"] if item["caseId"] == "cross-block-partition")
        report = evaluate_case(self.fixture, case, _public_prediction(case))
        self.assertTrue(report["structureValid"])
        self.assertEqual(report["boundary"]["f1"], 1.0)
        self.assertEqual(report["coverage"], 1.0)

    def test_runner_output_is_evaluated_without_sending_gold(self):
        captured = []

        def fake_run(command, *, input, capture_output, text, timeout, check):
            captured.append(json.loads(input))
            case = captured[-1]["caseId"]
            fixture_case = next(item for item in self.fixture["cases"] if item["caseId"] == case)
            payload = {"schemaVersion": 1, "status": "completed", "predictions": [_public_prediction(fixture_case)], "metadata": {"predictionSource": "external_runner"}}
            return type("Completed", (), {"returncode": 0, "stdout": json.dumps(payload), "stderr": ""})()

        with patch("cross_scope_eval.subprocess.run", side_effect=fake_run):
            report = run_evaluation(self.fixture, ["fake-runner"], "openaiResponses")
        self.assertEqual(report["status"], "completed")
        self.assertEqual(report["caseCount"], 4)
        self.assertEqual(report["structuralValidCount"], 4)
        self.assertEqual(len(captured), 4)
        for request in captured:
            public_fixture = request["semanticFixtures"][0]["fixture"]
            self.assertNotIn("gold", public_fixture)
            self.assertNotIn("negativeCases", public_fixture)
            self.assertNotIn("semanticRationale", public_fixture)

    def test_not_assessed_external_runner_is_not_converted_to_quality(self):
        def fake_run(*_args, **_kwargs):
            payload = {"status": "not_assessed", "reason": "missing endpoint", "predictions": []}
            return type("Completed", (), {"returncode": 0, "stdout": json.dumps(payload), "stderr": ""})()

        with patch("cross_scope_eval.subprocess.run", side_effect=fake_run):
            with self.assertRaisesRegex(CrossScopeError, "missing endpoint") as context:
                run_evaluation(self.fixture, ["fake-runner"], "openaiResponses")
        self.assertEqual(context.exception.status, "not_assessed")
        self.assertEqual(len(context.exception.details), 4)
        self.assertTrue(all(item["status"] == "not_assessed" for item in context.exception.details))
        self.assertTrue(all(item["attempts"] == 1 for item in context.exception.details))

    def test_case_failure_report_keeps_successful_cases_and_failed_none_case(self):
        def fake_run(_command, *, input, capture_output, text, timeout, check):
            request = json.loads(input)
            case = next(item for item in self.fixture["cases"] if item["caseId"] == request["caseId"])
            prediction = _public_prediction(case)
            if case["family"] == "none":
                prediction["children"] = [{
                    "childId": "invalid-none-child",
                    "spans": [{"documentId": "doc-beta", "blockId": "doc-beta:block-2", "segmentIds": ["beta-3"]}],
                    "evidenceSegmentIds": ["beta-3"],
                }]
            payload = {"schemaVersion": 1, "status": "completed", "predictions": [prediction]}
            return type("Completed", (), {"returncode": 0, "stdout": json.dumps(payload), "stderr": ""})()

        with patch("cross_scope_eval.subprocess.run", side_effect=fake_run):
            with self.assertRaisesRegex(CrossScopeError, "1 of 4") as context:
                run_evaluation(self.fixture, ["fake-runner"], "openaiResponses")
        details = {item["caseId"]: item for item in context.exception.details}
        self.assertEqual(details["cross-block-partition"]["status"], "completed")
        self.assertEqual(details["single-topic-none"]["status"], "failed")
        self.assertEqual(details["single-topic-none"]["errorCode"], "PREDICTION_STRUCTURE_INVALID")

    def test_invalid_cross_scope_prediction_fails_closed(self):
        case = self.fixture["cases"][0]
        invalid = _public_prediction(case)
        invalid["children"][0]["spans"][0]["segmentIds"] = ["unknown-segment"]
        with self.assertRaisesRegex(CrossScopeError, "unknown segment"):
            evaluate_case(self.fixture, case, invalid)

    def test_compact_child_is_normalized_only_with_explicit_public_ownership(self):
        case = next(item for item in self.fixture["cases"] if item["caseId"] == "cross-block-partition")
        canonical = _public_prediction(case)
        compact = copy.deepcopy(canonical)
        compact["children"] = [
            {
                "childId": child["childId"],
                "documentId": child["spans"][0]["documentId"],
                "blockId": child["spans"][0]["blockId"],
                "segmentIds": child["spans"][0]["segmentIds"],
                "evidenceSegmentIds": child["evidenceSegmentIds"],
            }
            for child in canonical["children"]
        ]
        report = evaluate_case(self.fixture, case, compact)
        self.assertTrue(report["structureValid"])
        self.assertEqual(report["boundary"]["f1"], 1.0)

        invalid = copy.deepcopy(compact)
        invalid["children"][0]["documentId"] = "doc-beta"
        with self.assertRaisesRegex(CrossScopeError, "escapes its public document/block"):
            evaluate_case(self.fixture, case, invalid)


if __name__ == "__main__":
    unittest.main()
