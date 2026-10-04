#!/usr/bin/env python3

import hashlib
import json
from copy import deepcopy
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from evaluate import evaluate_records, oracle_predictions, read_jsonl
from review import EvidenceError, summarize


ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "crates/application/tests/fixtures/assistant/synthetic-corpus.jsonl"
MANIFEST = ROOT / "scripts/assistant-eval/review-manifest.json"


def _write_evidence(directory: Path, *, oracle: bool = False) -> tuple[Path, Path]:
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    records = read_jsonl(CORPUS)
    sample_ids = [sample["scenarioId"] for sample in manifest["samples"]]
    by_id = {record["scenarioId"]: record for record in records}
    selected = [by_id[scenario_id] for scenario_id in sample_ids]
    predictions = deepcopy(oracle_predictions(selected))
    if not oracle:
        predictions[0]["children"][0]["evidence_segment_ids"] = [
            predictions[0]["children"][0]["start_segment_id"]
        ]
    prediction_path = directory / "prediction.jsonl"
    prediction_path.write_text(
        "".join(json.dumps(prediction, sort_keys=True) + "\n" for prediction in predictions),
        encoding="utf-8",
    )
    report = evaluate_records(selected, predictions)
    report["corpusSha256"] = hashlib.sha256(CORPUS.read_bytes()).hexdigest()
    report["predictionSha256"] = hashlib.sha256(prediction_path.read_bytes()).hexdigest()
    report["predictionSource"] = "oracle-gold-copy" if oracle else "external"
    report_path = directory / "report.json"
    report_path.write_text(json.dumps(report, sort_keys=True), encoding="utf-8")
    return prediction_path, report_path


class ReviewTests(unittest.TestCase):
    def setUp(self):
        self.manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))

    def test_manifest_decisions_are_not_quality_evidence(self):
        result = summarize(self.manifest)
        self.assertEqual(result["sampleCount"], 24)
        self.assertEqual(result["declaredDecisions"], {"pass": 24})
        self.assertEqual(result["qualityStatus"], "not_assessed")

    def test_oracle_is_structural_regression_not_quality_pass(self):
        with tempfile.TemporaryDirectory() as temporary:
            prediction, report = _write_evidence(Path(temporary), oracle=True)
            result = summarize(
                self.manifest,
                corpus_path=CORPUS,
                prediction_path=prediction,
                report_path=report,
            )
        self.assertEqual(result["qualityStatus"], "structural_regression")
        self.assertTrue(result["oracleCopy"])
        self.assertEqual(result["metrics"]["boundaryF1"], 1.0)

    def test_independent_evidence_derives_quality_from_recomputed_metrics(self):
        with tempfile.TemporaryDirectory() as temporary:
            prediction, report = _write_evidence(Path(temporary))
            result = summarize(
                self.manifest,
                corpus_path=CORPUS,
                prediction_path=prediction,
                report_path=report,
            )
        self.assertEqual(result["qualityStatus"], "passed")
        self.assertFalse(result["oracleCopy"])
        self.assertTrue(result["sampleCoverage"])
        self.assertEqual(set(result["artifactHashes"]), {"corpusSha256", "predictionSha256", "reportSha256"})

    def test_prediction_coverage_failure_is_explicit(self):
        with tempfile.TemporaryDirectory() as temporary:
            prediction, report = _write_evidence(Path(temporary))
            lines = prediction.read_text(encoding="utf-8").splitlines()
            prediction.write_text("\n".join(lines[:-1]) + "\n", encoding="utf-8")
            with self.assertRaisesRegex(EvidenceError, "sample coverage mismatch"):
                summarize(
                    self.manifest,
                    corpus_path=CORPUS,
                    prediction_path=prediction,
                    report_path=report,
                )

    def test_report_metric_must_match_recomputation(self):
        with tempfile.TemporaryDirectory() as temporary:
            prediction, report = _write_evidence(Path(temporary))
            payload = json.loads(report.read_text(encoding="utf-8"))
            payload["boundaryF1"] = 0.0
            report.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")
            with self.assertRaisesRegex(EvidenceError, "report metric boundaryF1"):
                summarize(
                    self.manifest,
                    corpus_path=CORPUS,
                    prediction_path=prediction,
                    report_path=report,
                )

    def test_partial_evidence_arguments_fail_clearly(self):
        with self.assertRaisesRegex(EvidenceError, "supplied together"):
            summarize(self.manifest, corpus_path=CORPUS)


if __name__ == "__main__":
    unittest.main()
