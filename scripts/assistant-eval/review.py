#!/usr/bin/env python3
"""Validate deterministic review inputs and derive quality from eval evidence.

The review manifest is a sample registry. Its decision and notes fields are
deliberately non-authoritative: quality is derived only when an independent
corpus, prediction artifact, and evaluator report can be checked together.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from collections import Counter
from pathlib import Path
from typing import Any

from evaluate import evaluate_records, oracle_predictions, read_jsonl


SAMPLE_COUNT = 24
ALLOWED_MODES = {"partition", "extract", "none"}
REPORT_METRICS = (
    "exactMatchRate",
    "structuralValidityRate",
    "evidenceValidityRate",
    "hierarchyValidityRate",
    "partitionCoverageRate",
    "extractGapReportValidityRate",
    "boundaryF1",
    "Pk",
    "WindowDiff",
)

# These gates are evaluator checks, not values read from review-manifest.json.
# They are limited to metrics emitted by evaluate.py until semantic thresholds
# are defined by the plan's separate quality-matrix work.
QUALITY_THRESHOLDS = {
    "exactMatchRate": ("min", 0.80),
    "structuralValidityRate": ("min", 1.0),
    "evidenceValidityRate": ("min", 1.0),
    "hierarchyValidityRate": ("min", 1.0),
    "partitionCoverageRate": ("min", 1.0),
    "extractGapReportValidityRate": ("min", 1.0),
    "boundaryF1": ("min", 0.80),
    "Pk": ("max", 0.20),
    "WindowDiff": ("max", 0.20),
}


class EvidenceError(ValueError):
    """Raised when review evidence is missing, unbound, or inconsistent."""


def sha256_file(path: Path) -> str:
    try:
        data = path.read_bytes()
    except OSError as error:
        raise EvidenceError(f"cannot read {path}: {error}") from error
    return hashlib.sha256(data).hexdigest()


def _load_json(path: Path, label: str) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise EvidenceError(f"cannot read {label} {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise EvidenceError(f"{label} {path} is invalid JSON: {error}") from error


def _load_predictions(path: Path) -> list[dict[str, Any]]:
    """Accept evaluator JSONL and the equivalent JSON array/object wrapper."""
    try:
        raw = path.read_text(encoding="utf-8")
    except OSError as error:
        raise EvidenceError(f"cannot read prediction artifact {path}: {error}") from error
    try:
        decoded = json.loads(raw)
    except json.JSONDecodeError:
        try:
            predictions = read_jsonl(path)
        except (OSError, ValueError) as error:
            raise EvidenceError(f"prediction artifact {path} is invalid JSONL: {error}") from error
    else:
        if isinstance(decoded, list):
            predictions = decoded
        elif isinstance(decoded, dict) and isinstance(decoded.get("predictions"), list):
            predictions = decoded["predictions"]
        else:
            raise EvidenceError(
                f"prediction artifact {path} must be JSONL, an array, or an object with predictions"
            )
    if not all(isinstance(prediction, dict) for prediction in predictions):
        raise EvidenceError(f"prediction artifact {path} contains a non-object record")
    return predictions


def validate_manifest(manifest: dict[str, Any]) -> list[str]:
    """Validate the registry and return its ordered, non-authoritative sample IDs."""
    if not isinstance(manifest, dict):
        raise EvidenceError("review manifest must be a JSON object")
    if manifest.get("schemaVersion") != 1:
        raise EvidenceError("review manifest schemaVersion must be 1")
    if manifest.get("status") != "completed_deterministic":
        raise EvidenceError("review manifest status must be completed_deterministic")
    if manifest.get("reviewMethod") != "deterministic_synthetic_oracle":
        raise EvidenceError(
            "review manifest reviewMethod must be deterministic_synthetic_oracle"
        )
    corpus_sha256 = manifest.get("corpusSha256")
    if not isinstance(corpus_sha256, str) or len(corpus_sha256) != 64:
        raise EvidenceError("review manifest corpusSha256 must be a 64-character hash")
    try:
        int(corpus_sha256, 16)
    except ValueError as error:
        raise EvidenceError("review manifest corpusSha256 is not hexadecimal") from error

    samples = manifest.get("samples")
    if not isinstance(samples, list) or len(samples) != SAMPLE_COUNT:
        raise EvidenceError(f"review manifest must contain exactly {SAMPLE_COUNT} samples")
    if manifest.get("sampleCount") != SAMPLE_COUNT:
        raise EvidenceError(f"review manifest sampleCount must be {SAMPLE_COUNT}")
    sample_ids: list[str] = []
    for index, sample in enumerate(samples):
        if not isinstance(sample, dict):
            raise EvidenceError(f"review manifest samples[{index}] must be an object")
        scenario_id = sample.get("scenarioId")
        if not isinstance(scenario_id, str) or not scenario_id:
            raise EvidenceError(f"review manifest samples[{index}] has no scenarioId")
        if scenario_id in sample_ids:
            raise EvidenceError(f"review manifest contains duplicate scenarioId: {scenario_id}")
        sample_ids.append(scenario_id)
        if sample.get("expectedMode") not in ALLOWED_MODES:
            raise EvidenceError(
                f"review manifest sample {scenario_id} has unsupported expectedMode"
            )
        if sample.get("reviewer") is not None:
            raise EvidenceError(
                f"review manifest sample {scenario_id} must not require a human reviewer"
            )
        if sample.get("decision") is not None and sample["decision"] not in {
            "pass",
            "fail",
            "uncertain",
        }:
            raise EvidenceError(
                f"review manifest sample {scenario_id} has unsupported declared decision"
            )
    return sample_ids


def _records_for_samples(
    corpus_path: Path, manifest: dict[str, Any], sample_ids: list[str]
) -> tuple[list[dict[str, Any]], str]:
    corpus_sha256 = sha256_file(corpus_path)
    if corpus_sha256 != manifest["corpusSha256"]:
        raise EvidenceError(
            "corpus hash mismatch: "
            f"manifest={manifest['corpusSha256']} actual={corpus_sha256}"
        )
    try:
        records = read_jsonl(corpus_path)
    except (OSError, ValueError) as error:
        raise EvidenceError(f"cannot load corpus {corpus_path}: {error}") from error
    record_ids = [record.get("scenarioId") for record in records]
    if any(not isinstance(scenario_id, str) or not scenario_id for scenario_id in record_ids):
        raise EvidenceError("corpus must contain non-empty string scenarioId values")
    by_id = {record["scenarioId"]: record for record in records}
    if len(by_id) != len(records):
        raise EvidenceError("corpus must contain unique non-empty scenarioId values")
    missing = [scenario_id for scenario_id in sample_ids if scenario_id not in by_id]
    if missing:
        raise EvidenceError(f"corpus is missing review samples: {missing}")
    return [by_id[scenario_id] for scenario_id in sample_ids], corpus_sha256


def _assert_metric_matches(report: dict[str, Any], measured: dict[str, Any]) -> None:
    for metric in REPORT_METRICS:
        reported = report.get(metric)
        actual = measured.get(metric)
        if not isinstance(reported, (int, float)) or isinstance(reported, bool):
            raise EvidenceError(f"report is missing numeric metric {metric}")
        if not math.isclose(float(reported), float(actual), rel_tol=0.0, abs_tol=1e-12):
            raise EvidenceError(
                f"report metric {metric} does not match recomputed value: "
                f"reported={reported} actual={actual}"
            )


def _quality_failures(report: dict[str, Any]) -> list[str]:
    failures = []
    for metric, (direction, threshold) in QUALITY_THRESHOLDS.items():
        value = float(report[metric])
        if direction == "min" and value < threshold:
            failures.append(f"{metric}={value} < {threshold}")
        if direction == "max" and value > threshold:
            failures.append(f"{metric}={value} > {threshold}")
    return failures


def evaluate_evidence(
    manifest: dict[str, Any],
    corpus_path: Path,
    prediction_path: Path,
    report_path: Path,
) -> dict[str, Any]:
    """Bind and recompute independent evidence for the manifest sample set."""
    sample_ids = validate_manifest(manifest)
    records, corpus_sha256 = _records_for_samples(corpus_path, manifest, sample_ids)
    predictions = _load_predictions(prediction_path)
    prediction_sha256 = sha256_file(prediction_path)
    prediction_ids = [prediction.get("scenarioId") for prediction in predictions]
    if any(not isinstance(scenario_id, str) or not scenario_id for scenario_id in prediction_ids):
        raise EvidenceError("prediction artifact contains a missing or invalid scenarioId")
    if len(set(prediction_ids)) != len(prediction_ids):
        raise EvidenceError("prediction artifact contains duplicate scenarioId values")
    missing_predictions = sorted(set(sample_ids) - set(prediction_ids))
    unexpected_predictions = sorted(set(prediction_ids) - set(sample_ids))
    if missing_predictions or unexpected_predictions:
        raise EvidenceError(
            "prediction sample coverage mismatch: "
            f"missing={missing_predictions} unexpected={unexpected_predictions}"
        )

    report = _load_json(report_path, "evaluation report")
    if not isinstance(report, dict):
        raise EvidenceError("evaluation report must be a JSON object")
    if report.get("corpusSha256") != corpus_sha256:
        raise EvidenceError(
            "report corpusSha256 does not match the checked corpus: "
            f"reported={report.get('corpusSha256')} actual={corpus_sha256}"
        )
    if report.get("predictionSha256") != prediction_sha256:
        raise EvidenceError(
            "report predictionSha256 does not match the prediction artifact: "
            f"reported={report.get('predictionSha256')} actual={prediction_sha256}"
        )
    examples = report.get("examples")
    if not isinstance(examples, list):
        raise EvidenceError("evaluation report must retain examples for sample-coverage checks")
    example_ids = [example.get("scenarioId") for example in examples if isinstance(example, dict)]
    if len(example_ids) != len(examples) or len(set(example_ids)) != len(example_ids):
        raise EvidenceError("evaluation report examples must have unique scenarioId values")
    if sorted(example_ids) != sorted(sample_ids):
        raise EvidenceError(
            "evaluation report sample coverage mismatch: "
            f"expected={sorted(sample_ids)} actual={sorted(example_ids)}"
        )
    if report.get("exampleCount") != len(sample_ids):
        raise EvidenceError(f"evaluation report exampleCount must be {len(sample_ids)}")

    measured = evaluate_records(records, predictions)
    _assert_metric_matches(report, measured)
    if report.get("missingScenarioIds") != measured["missingScenarioIds"]:
        raise EvidenceError("report missingScenarioIds does not match recomputed coverage")
    if report.get("unexpectedScenarioIds") != measured["unexpectedScenarioIds"]:
        raise EvidenceError("report unexpectedScenarioIds does not match recomputed coverage")

    evaluator_fields = {"scenarioId", "mode", "children", "uncovered_ranges", "second_pass"}
    oracle_projection = [
        {key: prediction[key] for key in evaluator_fields if key in prediction}
        for prediction in predictions
    ]
    expected_oracle_projection = [
        {key: prediction[key] for key in evaluator_fields if key in prediction}
        for prediction in oracle_predictions(records)
    ]
    oracle_copy = sorted(oracle_projection, key=lambda prediction: prediction["scenarioId"]) == sorted(
        expected_oracle_projection, key=lambda prediction: prediction["scenarioId"]
    )
    prediction_source = report.get("predictionSource")
    if not isinstance(prediction_source, str) or not prediction_source:
        raise EvidenceError("evaluation report must identify predictionSource")
    is_external = prediction_source == "external" or prediction_source.startswith("external:")
    if oracle_copy or not is_external:
        quality_status = "structural_regression"
        failures: list[str] = []
    else:
        failures = _quality_failures(report)
        quality_status = "passed" if not failures else "failed"

    return {
        "qualityStatus": quality_status,
        "sampleCount": len(sample_ids),
        "sampleCoverage": True,
        "artifactHashes": {
            "corpusSha256": corpus_sha256,
            "predictionSha256": prediction_sha256,
            "reportSha256": sha256_file(report_path),
        },
        "predictionSource": prediction_source,
        "oracleCopy": oracle_copy,
        "metrics": {metric: measured[metric] for metric in REPORT_METRICS},
        "failedCriteria": failures,
    }


def summarize(
    manifest: dict[str, Any],
    *,
    corpus_path: Path | None = None,
    prediction_path: Path | None = None,
    report_path: Path | None = None,
) -> dict[str, Any]:
    """Summarize the registry without treating decisions as quality evidence."""
    sample_ids = validate_manifest(manifest)
    evidence_paths = (corpus_path, prediction_path, report_path)
    if any(path is not None for path in evidence_paths) and not all(
        path is not None for path in evidence_paths
    ):
        raise EvidenceError("corpus, prediction, and report must be supplied together")
    declared = Counter(
        sample.get("decision") or "pending" for sample in manifest["samples"]
    )
    result: dict[str, Any] = {
        "schemaVersion": manifest["schemaVersion"],
        "sampleCount": len(sample_ids),
        "manifestStatus": manifest["status"],
        "declaredDecisions": dict(sorted(declared.items())),
        "qualityStatus": "not_assessed",
        "qualityReason": "independent corpus, prediction, and report evidence is required",
    }
    if all(path is not None for path in evidence_paths):
        result.pop("qualityReason")
        result.update(
            evaluate_evidence(
                manifest,
                corpus_path,  # type: ignore[arg-type]
                prediction_path,  # type: ignore[arg-type]
                report_path,  # type: ignore[arg-type]
            )
        )
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--prediction", type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    try:
        manifest = _load_json(args.manifest, "review manifest")
        result = summarize(
            manifest,
            corpus_path=args.corpus,
            prediction_path=args.prediction,
            report_path=args.report,
        )
    except (EvidenceError, TypeError) as error:
        parser.error(str(error))
    print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
