#!/usr/bin/env python3

import json
import os
import subprocess
import sys
import tempfile
import unittest
from copy import deepcopy
from argparse import Namespace
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from run_matrix import (
    CASES,
    PROTOCOLS,
    MatrixError,
    build_request,
    load_json,
    normalize_runner_output,
    run_matrix,
    run_one,
    select_case_records,
    validate_model_matrix,
    validate_prediction_contract,
    validate_sources,
    WritebackError,
)
from evaluate import read_jsonl
from verify import verify_model_matrix


ROOT = Path(__file__).resolve().parents[2]
MATRIX = ROOT / "scripts/assistant-eval/model-matrix.json"
SOURCES = ROOT / "scripts/assistant-eval/sources.json"
CORPUS = ROOT / "crates/application/tests/fixtures/assistant/synthetic-corpus.jsonl"
HOLDOUT = ROOT / "scripts/assistant-eval/review-manifest.json"


class MatrixRunnerTests(unittest.TestCase):
    def _partition_runner_output(self, request):
        record = request["records"][0]
        segments = record["record"]["state"]["results"]["transcript"]
        prediction = {
            "scenarioId": record["scenarioId"],
            "mode": "partition",
            "children": [{
                "start_segment_id": segments[0]["id"],
                "end_segment_id": segments[-1]["id"],
                "evidence_segment_ids": [segments[0]["id"]],
                "title": "External topic",
                "summary": "Evidence-backed topic",
                "tags": ["external"],
            }],
            "uncovered_ranges": [],
        }
        return subprocess.CompletedProcess(
            args=["fixture-runner"],
            returncode=0,
            stdout=json.dumps({"status": "completed", "predictions": [prediction], "metadata": {}}),
            stderr="",
        )

    def test_explicit_writeback_publishes_real_receipt_metadata(self):
        matrix = load_json(MATRIX)
        records = read_jsonl(CORPUS)[:1]
        with tempfile.TemporaryDirectory() as temporary, patch("run_matrix.subprocess.run") as run, patch(
            "run_matrix.writeback_predictions",
            return_value={
                "mutationReceipts": [{"mutationId": "real-mutation", "affectedDocuments": []}],
                "readback": [{"documentId": "doc", "metadata": {"revision": 1}, "markdown": "# doc\n"}],
                "evidence": [],
            },
        ) as writeback:
            run.side_effect = lambda command, **kwargs: self._partition_runner_output(json.loads(kwargs["input"]))
            result = run_one(
                matrix,
                {"status": "verified", "manifestSha256": "a" * 64, "baselineIds": ["no-split", "equal-length"]},
                records,
                "b" * 64,
                "c" * 64,
                "openaiResponses",
                "partition",
                "fixture-runner",
                Path(temporary) / "runs",
                {"MEETNOTE_ASSISTANT_EVAL_MODEL": "fixture", "OPENAI_API_KEY": "fixture"},
                None,
                "fixture-cli",
                Path(temporary) / "packages",
            )
            self.assertEqual(result["status"], "completed")
            writeback.assert_called_once()
            receipt = json.loads((Path(temporary) / "runs/openaiResponses/partition/receipt.json").read_text())
            self.assertEqual(receipt["mutationReceipts"][0]["mutationId"], "real-mutation")

    def test_writeback_failure_is_explicit_and_never_completed(self):
        matrix = load_json(MATRIX)
        records = read_jsonl(CORPUS)[:1]
        with tempfile.TemporaryDirectory() as temporary, patch("run_matrix.subprocess.run") as run, patch(
            "run_matrix.writeback_predictions",
            side_effect=WritebackError("WORKSPACE_READBACK_INVALID", "fixture failure"),
        ):
            run.side_effect = lambda command, **kwargs: self._partition_runner_output(json.loads(kwargs["input"]))
            result = run_one(
                matrix,
                {"status": "verified", "manifestSha256": "a" * 64, "baselineIds": ["no-split", "equal-length"]},
                records,
                "b" * 64,
                "c" * 64,
                "openaiResponses",
                "partition",
                "fixture-runner",
                Path(temporary) / "runs",
                {"MEETNOTE_ASSISTANT_EVAL_MODEL": "fixture", "OPENAI_API_KEY": "fixture"},
                None,
                "fixture-cli",
                Path(temporary) / "packages",
            )
            self.assertEqual(result["status"], "failed")
            receipt = json.loads((Path(temporary) / "runs/openaiResponses/partition/receipt.json").read_text())
            self.assertEqual(receipt["errorCode"], "WORKSPACE_WRITEBACK_FAILED")
            self.assertEqual(receipt["status"], "failed")
    def test_matrix_keeps_verify_compatibility_and_declares_fixed_contract(self):
        verify_model_matrix()
        matrix = load_json(MATRIX)
        validate_model_matrix(matrix)
        self.assertEqual(matrix["protocols"], list(PROTOCOLS))
        self.assertEqual(matrix["cases"], list(CASES))
        self.assertEqual(matrix["splits"]["synthetic"]["holdout"]["sampleCount"], 24)
        self.assertEqual(matrix["thresholds"]["boundaries"]["boundaryF1Min"], 0.8)
        self.assertFalse(matrix["runContract"]["oraclePredictionsAllowed"])

    def test_sources_require_license_checksums_and_baselines(self):
        sources = load_json(SOURCES)
        with tempfile.TemporaryDirectory() as temporary:
            report = validate_sources(sources, Path(temporary))
        self.assertEqual(report["status"], "pending_external_source")
        self.assertEqual(set(report["datasetIds"]), {"qmsum", "ami"})
        self.assertEqual(report["baselineIds"], ["no-split", "equal-length"])
        self.assertEqual(len(report["missingFiles"]), 6)

    def test_sources_reject_unpinned_baseline_and_noncanonical_checksum(self):
        sources = load_json(SOURCES)
        with tempfile.TemporaryDirectory() as temporary:
            broken_baseline = deepcopy(sources)
            broken_baseline["datasets"][0]["baseline"]["comparison"] = "trust the candidate"
            with self.assertRaisesRegex(MatrixError, "baseline comparison"):
                validate_sources(broken_baseline, Path(temporary))

            broken_checksum = deepcopy(sources)
            broken_checksum["datasets"][0]["files"][0]["sha256"] = "A" * 64
            with self.assertRaisesRegex(MatrixError, "SHA-256"):
                validate_sources(broken_checksum, Path(temporary))

    def test_missing_model_writes_pending_artifacts_without_predictions(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "runs"
            args = Namespace(
                matrix=MATRIX,
                sources=SOURCES,
                corpus=CORPUS,
                holdout=HOLDOUT,
                source_root=Path(temporary) / "missing-source-cache",
                output_dir=output,
                runner=None,
                protocol="openaiResponses",
                case="none",
                timeout=None,
            )
            environment = {
                key: value
                for key, value in os.environ.items()
                if key not in {"MEETNOTE_ASSISTANT_EVAL_MODEL", "OPENAI_API_KEY", "MEETNOTE_ASSISTANT_EVAL_API_KEY"}
            }
            with patch.dict(os.environ, environment, clear=True):
                report = run_matrix(args)
            self.assertEqual(report["runCount"], 1)
            self.assertEqual(report["statuses"]["pending_external_model"], 1)
            run_dir = output / "openaiResponses" / "none"
            prediction = json.loads((run_dir / "prediction.jsonl").read_text(encoding="utf-8"))
            self.assertEqual(prediction["status"], "pending_external_model")
            self.assertEqual(prediction["predictions"], [])
            receipt = json.loads((run_dir / "receipt.json").read_text(encoding="utf-8"))
            self.assertEqual(receipt["status"], "pending_external_model")
            readback = json.loads((run_dir / "readback.json").read_text(encoding="utf-8"))
            self.assertEqual(readback["status"], "pending_external_model")
            self.assertEqual(readback["predictionSha256"], readback["predictionReadback"]["sha256"])
            self.assertIn("mutationReceipts", readback)
            run_receipt = json.loads((run_dir / "receipt.json").read_text(encoding="utf-8"))
            self.assertEqual(run_receipt["predictionSha256"], readback["predictionSha256"])
            self.assertEqual(run_receipt["readbackSha256"], __import__("hashlib").sha256((run_dir / "readback.json").read_bytes()).hexdigest())

    def test_validator_prediction_requires_narrow_non_retrying_correction(self):
        matrix = load_json(MATRIX)
        records = read_jsonl(CORPUS)[:2]
        predictions = [
            {
                "scenarioId": record["scenarioId"],
                "mutations": [],
                "correction": {
                    "errorCode": "RANGE_OVERLAP",
                    "correctedOperation": {"operation": "read_document"},
                    "retry": False,
                },
            }
            for record in records
        ]
        result = validate_prediction_contract(
            "validator_correction", matrix["tasks"]["validator_correction"], records, predictions, matrix
        )
        self.assertEqual(result["predictionScenarioCoverage"], 1.0)
        predictions[0]["correction"]["retry"] = True
        with self.assertRaisesRegex(MatrixError, "retry=false"):
            validate_prediction_contract(
                "validator_correction", matrix["tasks"]["validator_correction"], records, predictions, matrix
            )

    def test_organize_requires_typed_non_empty_mutation_and_ask_exact_citations(self):
        matrix = load_json(MATRIX)
        record = read_jsonl(CORPUS)[0]
        organize = [{"scenarioId": record["scenarioId"], "mutations": []}]
        with self.assertRaisesRegex(MatrixError, "organize mutations must be non-empty"):
            validate_prediction_contract("organize_automatic", matrix["tasks"]["organize_automatic"], [record], organize, matrix)
        valid_update = [{
            "scenarioId": record["scenarioId"],
            "mutations": [{
                "op": "update_document",
                "fields": {"document_status": "ready"},
                "evidenceSegmentIds": [record["record"]["state"]["results"]["transcript"][0]["id"]],
                "description": "Keep this explanation outside the patch.",
            }],
        }]
        result = validate_prediction_contract("organize_automatic", matrix["tasks"]["organize_automatic"], [record], valid_update, matrix)
        self.assertEqual(result["predictionScenarioCoverage"], 1.0)
        both_aliases = [{
            "scenarioId": record["scenarioId"],
            "mutations": [{"op": "update_document", "fields": {"document_status": "ready", "documentStatus": "ready"}}],
        }]
        with self.assertRaisesRegex(MatrixError, "both document_status and documentStatus"):
            validate_prediction_contract("organize_automatic", matrix["tasks"]["organize_automatic"], [record], both_aliases, matrix)
        invalid_range = [{
            "scenarioId": record["scenarioId"],
            "mutations": [{"op": "update_document", "fields": {"title": "bad"}, "start_segment_id": "segment-1"}],
        }]
        with self.assertRaisesRegex(MatrixError, "contains range fields"):
            validate_prediction_contract("organize_automatic", matrix["tasks"]["organize_automatic"], [record], invalid_range, matrix)
        ask = [{"scenarioId": record["scenarioId"], "answer": "answer", "citations": ["not-a-supplied-segment"]}]
        with self.assertRaisesRegex(MatrixError, "exact supplied segment IDs"):
            validate_prediction_contract("ask_direct", matrix["tasks"]["ask_direct"], [record], ask, matrix)

    def test_runner_not_assessed_and_failed_envelopes_are_not_completed(self):
        for status in ("not_assessed", "failed"):
            with self.subTest(status=status):
                self.assertEqual(
                    normalize_runner_output(
                        {"status": status, "predictions": [], "reason": "fixture"}
                    ),
                    (status, [], "fixture"),
                )
                with self.assertRaisesRegex(MatrixError, "must not contain predictions"):
                    normalize_runner_output(
                        {"status": status, "predictions": [{"scenarioId": "x"}]}
                    )

    def test_request_carries_scope_fixture_without_gold_or_negative_cases(self):
        matrix = load_json(MATRIX)
        records = read_jsonl(CORPUS)[:1]
        request = build_request(
            matrix,
            "openaiResponses",
            "partition",
            records,
            "a" * 64,
            {"status": "pending_external_source", "manifestSha256": "b" * 64, "baselineIds": ["no-split", "equal-length"]},
            "c" * 64,
        )
        fixture = request["semanticFixtures"][0]["fixture"]
        self.assertEqual(request["semanticFixtures"][0]["id"], "cross-scope")
        self.assertTrue(all("gold" not in case and "negativeCases" not in case for case in fixture["cases"]))
        self.assertTrue(all("semanticRationale" not in case for case in fixture["cases"]))

if __name__ == "__main__":
    unittest.main()
