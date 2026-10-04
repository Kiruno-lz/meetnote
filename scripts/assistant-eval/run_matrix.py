#!/usr/bin/env python3
"""Run the external Assistant semantic matrix without fabricating evidence.

The command is deliberately an adapter boundary: a caller-owned runner receives
one JSON request on stdin and must return predictions as JSON.  Without a real
runner, model, and credential it writes auditable pending artifacts only.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import shlex
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

from evaluate import evaluate_records, read_jsonl, validate_structure
from semantic_schema import SemanticSchemaError, validate_semantic_fixture
from writeback import WritebackConfig, WritebackError, writeback_predictions


ROOT = Path(__file__).resolve().parents[2]
SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_MATRIX = SCRIPT_DIR / "model-matrix.json"
DEFAULT_SOURCES = SCRIPT_DIR / "sources.json"
DEFAULT_CORPUS = ROOT / "crates/application/tests/fixtures/assistant/synthetic-corpus.jsonl"
DEFAULT_HOLDOUT = SCRIPT_DIR / "review-manifest.json"
DEFAULT_SOURCE_ROOT = ROOT / "eval-data/assistant/data"
DEFAULT_OUTPUT = ROOT / "eval-data/assistant/runs"

PROTOCOLS = (
    "openaiChatCompletions",
    "anthropicMessages",
    "openaiResponses",
)
CASES = (
    "ask_direct",
    "ask_second_read",
    "ask_unknown",
    "organize_automatic",
    "organize_confirm_each",
    "validator_correction",
    "partition",
    "extract",
    "none",
    "nested_second_pass",
)
ALLOWED_STATUSES = {
    "pending_external_model",
    "pending_external_source",
    "not_assessed",
    "completed",
    "failed",
}


class MatrixError(ValueError):
    """Raised for a malformed matrix, source manifest, or runner response."""


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode(
        "utf-8"
    )


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise MatrixError(f"cannot load JSON {path}: {error}") from error
    if not isinstance(value, dict):
        raise MatrixError(f"JSON root must be an object: {path}")
    return value


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def validate_model_matrix(matrix: dict[str, Any]) -> None:
    if matrix.get("schemaVersion") != 1:
        raise MatrixError("model matrix schemaVersion must be 1")
    if matrix.get("protocols") != list(PROTOCOLS):
        raise MatrixError("model matrix protocols do not match the three fixed protocols")
    if matrix.get("cases") != list(CASES):
        raise MatrixError("model matrix cases do not match the fixed semantic task set")
    if matrix.get("requiredEvidence") != [
        "predictionFile",
        "runMetadata",
        "structuredErrorOrReadback",
        "mutationReceipts",
    ]:
        raise MatrixError("model matrix requiredEvidence is incomplete or reordered")
    run_contract = matrix.get("runContract")
    if not isinstance(run_contract, dict):
        raise MatrixError("model matrix runContract is missing")
    for field, required in {
        "metadataRequired": ["runId", "caseId", "model", "promptSha256", "budget", "resources", "elapsedMs", "predictionSource"],
        "receiptRequired": ["runId", "requestSha256", "predictionSha256", "readbackSha256", "mutationReceipts"],
        "readbackRequired": ["runId", "predictionSha256", "metadataSha256", "mutationReceipts", "validation"],
    }.items():
        if run_contract.get(field) != required:
            raise MatrixError(f"run contract {field} is incomplete or reordered")
    if matrix.get("status") != "pending_external_model":
        raise MatrixError("checked-in matrix must remain pending_external_model until real runs exist")
    if matrix.get("runs") != []:
        raise MatrixError("checked-in matrix runs must remain empty; run artifacts are external evidence")
    semantic_fixtures = matrix.get("semanticFixtures")
    if not isinstance(semantic_fixtures, list) or not semantic_fixtures:
        raise MatrixError("model matrix must declare semantic fixtures")
    for fixture in semantic_fixtures:
        if not isinstance(fixture, dict) or not isinstance(fixture.get("id"), str) or not isinstance(fixture.get("path"), str):
            raise MatrixError("semantic fixture declaration is invalid")
        if fixture.get("independentModelEvidence") is not False:
            raise MatrixError("semantic fixture must not claim independent model evidence")

    model = matrix.get("model")
    if not isinstance(model, dict) or model.get("fixed") is not True:
        raise MatrixError("matrix must pin a fixed requested model profile")
    if not isinstance(model.get("requestedModel"), str) or not model["requestedModel"]:
        raise MatrixError("matrix requestedModel is missing")
    if not isinstance(model.get("modelEnv"), str) or not model["modelEnv"]:
        raise MatrixError("matrix modelEnv is missing")
    credentials = model.get("credentialEnvByProtocol")
    if not isinstance(credentials, dict) or set(credentials) != set(PROTOCOLS):
        raise MatrixError("matrix credential environment mapping is incomplete")
    if any(not isinstance(value, list) or not value for value in credentials.values()):
        raise MatrixError("every protocol must have at least one credential environment name")

    prompt = matrix.get("prompt")
    if not isinstance(prompt, dict) or prompt.get("id") != "assistant-semantic-eval-v1":
        raise MatrixError("matrix prompt id is not fixed")
    if prompt.get("version") != 1 or not isinstance(prompt.get("system"), str):
        raise MatrixError("matrix prompt version/system is missing")
    templates = prompt.get("templates")
    if not isinstance(templates, dict) or set(templates) != {
        "ask",
        "organize",
        "validator",
        "partition",
        "extract",
        "none",
        "nested",
    }:
        raise MatrixError("matrix prompt templates do not cover all task families")

    tasks = matrix.get("tasks")
    if not isinstance(tasks, dict) or set(tasks) != set(CASES):
        raise MatrixError("matrix task definitions do not cover all cases")
    for case_id in CASES:
        task = tasks[case_id]
        if not isinstance(task, dict) or not isinstance(task.get("family"), str):
            raise MatrixError(f"task definition is invalid: {case_id}")
        if task.get("promptTemplate") not in templates:
            raise MatrixError(f"task prompt template is invalid: {case_id}")

    resources = matrix.get("resources")
    required_resources = {
        "maxToolRounds",
        "maxOutputTokens",
        "timeoutMs",
        "maxInputBytes",
        "maxPredictionBytes",
        "maxReadbackBytes",
        "maxMutationReceipts",
        "concurrency",
    }
    if not isinstance(resources, dict) or not required_resources <= set(resources):
        raise MatrixError("matrix resource limits are incomplete")
    if any(not isinstance(resources[key], int) or resources[key] <= 0 for key in required_resources):
        raise MatrixError("matrix resource limits must be positive integers")
    if resources["concurrency"] != 1:
        raise MatrixError("matrix runner must remain serial for traceable receipts")

    thresholds = matrix.get("thresholds")
    if not isinstance(thresholds, dict):
        raise MatrixError("matrix thresholds are missing")
    for group, fields in {
        "boundaries": ("boundaryF1Min", "pkMax", "windowDiffMax"),
        "coverage": ("predictionScenarioCoverageMin", "partitionCoverageMin", "extractGapValidityMin"),
        "citations": ("answerCitationCoverageMin",),
        "summaries": ("summaryEvidenceCoverageMin", "unsupportedFactCountMax"),
        "structure": ("structuralValidityMin", "evidenceValidityMin", "hierarchyValidityMin"),
    }.items():
        if not isinstance(thresholds.get(group), dict) or not set(fields) <= set(thresholds[group]):
            raise MatrixError(f"matrix {group} thresholds are incomplete")

    baselines = matrix.get("baselines")
    if not isinstance(baselines, list) or {item.get("id") for item in baselines} != {
        "no-split",
        "equal-length",
    }:
        raise MatrixError("matrix baselines must include no-split and equal-length")
    validator_contract = matrix.get("predictionContracts", {}).get("validator")
    if not isinstance(validator_contract, dict) or validator_contract.get("correctionRequired") != [
        "errorCode",
        "correctedOperation",
        "retry",
    ] or validator_contract.get("retryMustBe") is not False:
        raise MatrixError("validator contract must require a non-retrying correction")


def validate_sources(manifest: dict[str, Any], source_root: Path) -> dict[str, Any]:
    if manifest.get("schemaVersion") != 1:
        raise MatrixError("source manifest schemaVersion must be 1")
    verification = manifest.get("verification")
    if not isinstance(verification, dict) or verification.get("hashAlgorithm") != "sha256":
        raise MatrixError("source manifest must require sha256 verification")
    datasets = manifest.get("datasets")
    if not isinstance(datasets, list) or {item.get("id") for item in datasets} != {"qmsum", "ami"}:
        raise MatrixError("source manifest must contain exactly QMSum and AMI")

    relative_paths: set[str] = set()
    files: list[dict[str, Any]] = []
    missing: list[str] = []
    for dataset in datasets:
        dataset_id = dataset.get("id")
        if dataset.get("licenseStatus") != verification.get("allowedLicenseStatus"):
            raise MatrixError(f"dataset {dataset_id} is not license-approved")
        if not isinstance(dataset.get("license"), str) or not dataset["license"]:
            raise MatrixError(f"dataset {dataset_id} has no license")
        if not isinstance(dataset.get("version"), str) or not dataset["version"]:
            raise MatrixError(f"dataset {dataset_id} has no pinned version")
        if not isinstance(dataset.get("homepage"), str) or not dataset["homepage"].startswith("https://"):
            raise MatrixError(f"dataset {dataset_id} has no HTTPS homepage")
        if not isinstance(dataset.get("scope"), str) or not dataset["scope"]:
            raise MatrixError(f"dataset {dataset_id} has no declared scope")
        if not isinstance(dataset.get("splits"), dict) or not dataset["splits"]:
            raise MatrixError(f"dataset {dataset_id} has no split declaration")
        baseline = dataset.get("baseline")
        if not isinstance(baseline, dict) or baseline.get("evaluator") != "scripts/assistant-eval/evaluate.py":
            raise MatrixError(f"dataset {dataset_id} has no evaluate.py baseline")
        if set(baseline.get("required", [])) != {"no-split", "equal-length"}:
            raise MatrixError(f"dataset {dataset_id} baseline set is incomplete")
        if baseline.get("comparison") != "candidate must improve boundaryF1 and reduce Pk and WindowDiff":
            raise MatrixError(f"dataset {dataset_id} baseline comparison is not pinned")
        declared_files = dataset.get("files")
        if not isinstance(declared_files, list) or not declared_files:
            raise MatrixError(f"dataset {dataset_id} has no declared files")
        if not {item.get("kind") for item in declared_files} >= {"annotation", "license"}:
            raise MatrixError(f"dataset {dataset_id} must declare annotation and license files")
        if sum(item.get("kind") == "license" for item in declared_files) != 1:
            raise MatrixError(f"dataset {dataset_id} must declare exactly one license snapshot")
        for item in declared_files:
            relative = item.get("relativePath")
            digest = item.get("sha256")
            if not isinstance(relative, str) or not relative or relative in relative_paths:
                raise MatrixError(f"duplicate or invalid source relativePath: {relative}")
            relative_paths.add(relative)
            if item.get("url", "").startswith("https://") is False:
                raise MatrixError(f"source URL is not HTTPS: {relative}")
            if not isinstance(item.get("bytes"), int) or item["bytes"] <= 0:
                raise MatrixError(f"source byte declaration is invalid: {relative}")
            if not isinstance(digest, str) or len(digest) != 64 or digest != digest.lower():
                raise MatrixError(f"source SHA-256 declaration is invalid: {relative}")
            try:
                int(digest, 16)
            except ValueError as error:
                raise MatrixError(f"source SHA-256 is not hexadecimal: {relative}") from error
            path = source_root / relative
            evidence = {"dataset": dataset_id, "relativePath": relative, "expectedBytes": item["bytes"], "expectedSha256": digest}
            if not path.is_file():
                missing.append(relative)
                evidence["status"] = "missing"
            else:
                observed_bytes = path.stat().st_size
                observed_hash = sha256_file(path)
                if observed_bytes != item["bytes"] or observed_hash != digest:
                    raise MatrixError(
                        f"source checksum mismatch for {relative}: bytes={observed_bytes} sha256={observed_hash}"
                    )
                evidence.update({"status": "verified", "observedBytes": observed_bytes, "observedSha256": observed_hash})
            files.append(evidence)

    excluded = {item.get("id"): item for item in manifest.get("excludedDatasets", [])}
    superdialseg = excluded.get("superdialseg")
    if not isinstance(superdialseg, dict) or superdialseg.get("enabled") is not False:
        raise MatrixError("excluded SuperDialseg entry is missing or enabled")
    return {
        "status": "pending_external_source" if missing else "verified",
        "missingFiles": missing,
        "files": files,
        "datasetIds": [dataset["id"] for dataset in datasets],
        "baselineIds": ["no-split", "equal-length"],
        "manifestSha256": sha256_bytes(canonical_json(manifest)),
    }


def load_holdout(corpus_path: Path, holdout_path: Path) -> tuple[list[dict[str, Any]], str, str]:
    try:
        records = read_jsonl(corpus_path)
        holdout = load_json(holdout_path)
    except (OSError, ValueError, MatrixError) as error:
        raise MatrixError(f"cannot load synthetic holdout: {error}") from error
    corpus_hash = sha256_file(corpus_path)
    if holdout.get("corpusSha256") != corpus_hash:
        raise MatrixError("holdout manifest corpusSha256 does not match the corpus")
    sample_ids = [sample.get("scenarioId") for sample in holdout.get("samples", [])]
    if len(sample_ids) != 24 or len(set(sample_ids)) != 24 or any(not isinstance(value, str) for value in sample_ids):
        raise MatrixError("holdout manifest must contain 24 unique scenario IDs")
    by_id = {record.get("scenarioId"): record for record in records}
    if any(sample_id not in by_id for sample_id in sample_ids):
        raise MatrixError("holdout manifest references a scenario absent from the corpus")
    return [by_id[sample_id] for sample_id in sample_ids], corpus_hash, sha256_file(holdout_path)


def select_case_records(records: list[dict[str, Any]], task: dict[str, Any]) -> list[dict[str, Any]]:
    selection = task.get("selection")
    if not isinstance(selection, dict):
        return records
    selected = records
    if "goldMode" in selection:
        selected = [record for record in selected if record.get("gold", {}).get("allowed_mode") == selection["goldMode"]]
    if selection.get("excludeSecondPass"):
        selected = [record for record in selected if "second_pass" not in record.get("gold", {})]
    if selection.get("requiresSecondPass"):
        selected = [record for record in selected if "second_pass" in record.get("gold", {})]
    if not selected:
        raise MatrixError(f"case selection produced no holdout records: {task}")
    return selected


def public_record(record: dict[str, Any]) -> dict[str, Any]:
    value = copy.deepcopy(record)
    value.pop("gold", None)
    value.pop("negativeCase", None)
    return value


def protocol_credentials(matrix: dict[str, Any], protocol: str, environment: dict[str, str]) -> list[str]:
    names = matrix["model"]["credentialEnvByProtocol"][protocol]
    return [name for name in names if environment.get(name)]


def pending_reason(matrix: dict[str, Any], protocol: str, environment: dict[str, str], runner: str | None) -> str | None:
    model_env = matrix["model"]["modelEnv"]
    if not environment.get(model_env):
        return f"missing_model_environment:{model_env}"
    if not protocol_credentials(matrix, protocol, environment):
        names = ",".join(matrix["model"]["credentialEnvByProtocol"][protocol])
        return f"missing_credential_environment:{names}"
    if not runner:
        return "missing_external_runner"
    return None


def build_request(
    matrix: dict[str, Any], protocol: str, case_id: str, records: list[dict[str, Any]],
    corpus_hash: str, source_report: dict[str, Any], holdout_hash: str,
) -> dict[str, Any]:
    task = matrix["tasks"][case_id]
    prompt_template = matrix["prompt"]["templates"][task["promptTemplate"]]
    request = {
        "schemaVersion": 1,
        "protocol": protocol,
        "caseId": case_id,
        "taskFamily": task["family"],
        "requestedModel": matrix["model"]["requestedModel"],
        "prompt": {
            "id": matrix["prompt"]["id"],
            "version": matrix["prompt"]["version"],
            "system": matrix["prompt"]["system"],
            "task": prompt_template,
        },
        "budget": {
            "maxToolRounds": matrix["resources"]["maxToolRounds"],
            "maxOutputTokens": matrix["resources"]["maxOutputTokens"],
        },
        "resources": matrix["resources"],
        "holdout": {"manifestSha256": holdout_hash, "scenarioCount": len(records), "goldIncluded": False},
        "corpusSha256": corpus_hash,
        "sourceVerification": {
            "status": source_report["status"],
            "manifestSha256": source_report["manifestSha256"],
            "baselineIds": source_report["baselineIds"],
        },
        "records": [public_record(record) for record in records],
        "outputContract": matrix["predictionContracts"].get(task["family"], matrix["predictionContracts"]["all"]),
    }
    for fixture_declaration in matrix["semanticFixtures"]:
        fixture_path = ROOT / fixture_declaration["path"]
        try:
            fixture = load_json(fixture_path)
            summary = validate_semantic_fixture(fixture)
        except (OSError, MatrixError, SemanticSchemaError) as error:
            raise MatrixError(f"semantic fixture is invalid: {error}") from error
        required_cases = fixture_declaration.get("requiredCases", [])
        case_ids = [case["caseId"] for case in fixture["cases"]]
        if sorted(case_ids) != sorted(required_cases):
            raise MatrixError(f"semantic fixture {fixture_declaration['id']} case coverage is incomplete")
        # Gold, negative cases, and rationale are intentionally not sent to a model.
        public_fixture = copy.deepcopy(fixture)
        for case in public_fixture["cases"]:
            case.pop("gold", None)
            case.pop("negativeCases", None)
            case.pop("semanticRationale", None)
        request.setdefault("semanticFixtures", []).append(
            {"id": fixture_declaration["id"], "summary": summary, "fixture": public_fixture}
        )
    return request


def normalize_runner_output(value: Any) -> tuple[str, list[dict[str, Any]], str | None]:
    if isinstance(value, dict) and value.get("status") in {
        "pending_external_model",
        "not_assessed",
        "failed",
    }:
        predictions = value.get("predictions", [])
        if predictions not in (None, []) or value.get("prediction") is not None:
            raise MatrixError("runner non-completed status must not contain predictions")
        return value["status"], [], str(value.get("reason", "runner did not produce a prediction"))
    if isinstance(value, list):
        predictions = value
    elif isinstance(value, dict) and isinstance(value.get("predictions"), list):
        predictions = value["predictions"]
    elif isinstance(value, dict) and isinstance(value.get("prediction"), dict):
        predictions = [value["prediction"]]
    elif isinstance(value, dict) and isinstance(value.get("scenarioId"), str):
        predictions = [value]
    else:
        raise MatrixError("runner output must be a prediction array or an object with predictions")
    if not all(isinstance(item, dict) for item in predictions):
        raise MatrixError("runner predictions must all be JSON objects")
    for prediction in predictions:
        source = prediction.get("predictionSource")
        if isinstance(source, str) and "oracle" in source.lower():
            raise MatrixError("oracle-gold-copy is not an external model result")
    return "completed", predictions, None


def validate_prediction_contract(
    case_id: str, task: dict[str, Any], records: list[dict[str, Any]], predictions: list[dict[str, Any]], matrix: dict[str, Any]
) -> dict[str, Any]:
    expected_ids = [record["scenarioId"] for record in records]
    actual_ids = [prediction.get("scenarioId") for prediction in predictions]
    if any(not isinstance(value, str) for value in actual_ids):
        raise MatrixError("prediction contains a missing scenarioId")
    if len(actual_ids) != len(set(actual_ids)):
        raise MatrixError("prediction contains duplicate scenarioId values")
    if sorted(actual_ids) != sorted(expected_ids):
        raise MatrixError(
            f"prediction coverage mismatch: missing={sorted(set(expected_ids) - set(actual_ids))} unexpected={sorted(set(actual_ids) - set(expected_ids))}"
        )

    by_id = {record["scenarioId"]: record for record in records}
    family = task["family"]
    structural_valid = 0
    citation_complete = 0
    summary_complete = 0
    for prediction in predictions:
        required = matrix["predictionContracts"].get(family, matrix["predictionContracts"]["all"])["required"]
        missing = [key for key in required if key not in prediction]
        if missing:
            raise MatrixError(f"{case_id} prediction {prediction['scenarioId']} is missing {missing}")
        if family in {"partition", "extract", "none", "nested"}:
            record = by_id[prediction["scenarioId"]]
            segment_ids = [segment["id"] for segment in record["record"]["state"]["results"]["transcript"]]
            validation = validate_structure(
                segment_ids,
                prediction.get("mode"),
                prediction.get("children"),
                prediction.get("uncovered_ranges"),
                prediction.get("second_pass"),
            )
            if not validation.valid:
                raise MatrixError(f"{case_id} prediction {prediction['scenarioId']} is structurally invalid: {validation.errors}")
            structural_valid += 1
        if family == "ask":
            if not isinstance(prediction["answer"], str):
                raise MatrixError(f"{case_id} answer must be a string")
            citations = prediction["citations"]
            if not isinstance(citations, list) or not all(isinstance(item, str) and item for item in citations):
                raise MatrixError(f"{case_id} citations must be a non-empty string array")
            segment_ids = {segment["id"] for segment in by_id[prediction["scenarioId"]]["record"]["state"]["results"]["transcript"]}
            invalid_citations = sorted(set(citations) - segment_ids)
            if invalid_citations:
                raise MatrixError(f"{case_id} citations are not exact supplied segment IDs: {invalid_citations}")
            citation_complete += 1
        if family in {"organize", "validator"}:
            if not isinstance(prediction["mutations"], list):
                raise MatrixError(f"{case_id} mutations must be an array")
            if family == "organize":
                if not prediction["mutations"]:
                    raise MatrixError(f"{case_id} organize mutations must be non-empty")
                for index, mutation in enumerate(prediction["mutations"]):
                    if not isinstance(mutation, dict):
                        raise MatrixError(f"{case_id} organize mutation {index} must be an object")
                    range_fields = {"start_segment_id", "end_segment_id", "uncovered_ranges", "children", "spans", "segmentIds", "blockId", "mode", "parent_child_index", "second_pass", "secondPass", "uncoveredSpans"}
                    if set(mutation) & range_fields:
                        raise MatrixError(f"{case_id} organize mutation {index} contains range fields")
                    if "op" in mutation and mutation.get("op") != "update_document":
                        if mutation.get("op") not in {"set_title", "set_tags", "set_summary", "set_markdown", "set_document_status"} or "value" not in mutation:
                            raise MatrixError(f"{case_id} organize mutation {index} is not a typed mutation")
                        if set(mutation) - {"op", "value", "documentId", "evidence_segment_ids", "evidenceSegmentIds", "rationale", "semanticRationale", "description", "explanation"}:
                            raise MatrixError(f"{case_id} organize mutation {index} has unsupported fields")
                    else:
                        operation = mutation.get("operation", mutation.get("tool", mutation.get("op")))
                        patch = mutation.get("patch", mutation.get("changes", mutation.get("fields")))
                        if patch is None and operation == "update_document":
                            patch = mutation.get("value")
                        if operation not in (None, "update_document") or not isinstance(patch, dict) or not patch:
                            raise MatrixError(f"{case_id} organize mutation {index} requires a typed operation and non-empty patch")
                        if "document_status" in patch and "documentStatus" in patch:
                            raise MatrixError(f"{case_id} organize mutation {index} supplies both document_status and documentStatus")
                        if set(patch) - {"title", "tags", "summary", "markdown", "documentStatus", "document_status"}:
                            raise MatrixError(f"{case_id} organize mutation {index} patch has unsupported fields")
            if family == "validator":
                correction = prediction["correction"]
                if not isinstance(correction, dict):
                    raise MatrixError(f"{case_id} correction must be an object")
                required_correction = {"errorCode", "correctedOperation", "retry"}
                missing_correction = sorted(required_correction - set(correction))
                if missing_correction:
                    raise MatrixError(
                        f"{case_id} correction is missing {missing_correction}"
                    )
                if not isinstance(correction["errorCode"], str) or not correction["errorCode"]:
                    raise MatrixError(f"{case_id} correction errorCode must be non-empty")
                if not isinstance(correction["correctedOperation"], dict):
                    raise MatrixError(f"{case_id} correctedOperation must be an object")
                if correction["retry"] is not False:
                    raise MatrixError(f"{case_id} correction must set retry=false")
        if family == "nested":
            second_pass = prediction.get("second_pass")
            if not isinstance(second_pass, dict):
                raise MatrixError(f"{case_id} nested second_pass must be an object")
            if second_pass.get("mode") != "partition" or not isinstance(second_pass.get("children"), list) or not isinstance(second_pass.get("uncovered_ranges"), list):
                raise MatrixError(f"{case_id} nested second_pass requires mode=partition, children, and uncovered_ranges")
            if not isinstance(second_pass.get("parent_child_index"), int) and not isinstance(second_pass.get("parentChildId"), str):
                raise MatrixError(f"{case_id} nested second_pass requires parent_child_index or parentChildId")
            if prediction.get("summaryEvidenceSegmentIds"):
                summary_complete += 1

    thresholds = matrix["thresholds"]
    if family == "ask" and citation_complete / len(predictions) < thresholds["citations"]["answerCitationCoverageMin"]:
        raise MatrixError(f"{case_id} citation coverage is below the fixed threshold")
    if family in {"partition", "extract", "none", "nested"} and structural_valid / len(predictions) < thresholds["structure"]["structuralValidityMin"]:
        raise MatrixError(f"{case_id} structural validity is below the fixed threshold")
    return {
        "predictionScenarioCoverage": len(predictions) / len(records),
        "structuralValidCount": structural_valid,
        "citationCompleteCount": citation_complete,
        "summaryEvidenceCount": summary_complete,
    }


def run_one(
    matrix: dict[str, Any], source_report: dict[str, Any], records: list[dict[str, Any]],
    corpus_hash: str, holdout_hash: str, protocol: str, case_id: str, runner: str | None,
    output_dir: Path, environment: dict[str, str], timeout_override: float | None,
    writeback_cli: str | None, writeback_package_root: Path | None,
) -> dict[str, Any]:
    request = build_request(matrix, protocol, case_id, records, corpus_hash, source_report, holdout_hash)
    request_hash = sha256_bytes(canonical_json(request))
    run_id = f"{protocol}-{case_id}-{request_hash[:16]}"
    run_dir = output_dir / protocol / case_id
    run_dir.mkdir(parents=True, exist_ok=True)
    prediction_path = run_dir / "prediction.jsonl"
    metadata_path = run_dir / "metadata.json"
    receipt_path = run_dir / "receipt.json"
    readback_path = run_dir / "readback.json"
    started = time.monotonic()
    started_at = datetime.now(timezone.utc).isoformat()
    resources = matrix["resources"]
    reason = pending_reason(matrix, protocol, environment, runner)
    status = "pending_external_model" if reason else "completed"
    error_code: str | None = None
    predictions: list[dict[str, Any]] = []
    validation: dict[str, Any] = {"predictionScenarioCoverage": 0.0}
    runner_metadata: dict[str, Any] = {}

    if reason is None and source_report["status"] != "verified":
        status = "pending_external_source"
        reason = "source_manifest_files_not_verified"
    if reason is None:
        command = shlex.split(runner or "")
        if not command:
            status = "pending_external_model"
            reason = "missing_external_runner"
        else:
            try:
                completed = subprocess.run(
                    command,
                    input=json.dumps(request, ensure_ascii=False),
                    capture_output=True,
                    text=True,
                    timeout=timeout_override or resources["timeoutMs"] / 1000,
                    check=False,
                    env=environment,
                )
                if completed.returncode != 0:
                    status = "failed"
                    error_code = "EXTERNAL_RUNNER_EXITED"
                    reason = f"runner_exit_code:{completed.returncode}"
                elif len(completed.stdout.encode("utf-8")) > resources["maxPredictionBytes"]:
                    status = "failed"
                    error_code = "PREDICTION_SIZE_EXCEEDED"
                    reason = "runner output exceeds maxPredictionBytes"
                else:
                    runner_value = json.loads(completed.stdout)
                    if isinstance(runner_value, dict) and isinstance(runner_value.get("metadata"), dict):
                        runner_metadata = copy.deepcopy(runner_value["metadata"])
                    result_status, predictions, reason = normalize_runner_output(runner_value)
                    if result_status in {"pending_external_model", "not_assessed"}:
                        status = result_status
                        if isinstance(runner_value, dict) and runner_value.get("errorCode"):
                            error_code = str(runner_value["errorCode"])
                    elif result_status == "failed":
                        status = "failed"
                        error_code = (
                            str(runner_value.get("errorCode"))
                            if isinstance(runner_value, dict) and runner_value.get("errorCode")
                            else "EXTERNAL_RUNNER_FAILED"
                        )
                    else:
                        validation = validate_prediction_contract(
                            case_id, matrix["tasks"][case_id], records, predictions, matrix
                        )
                        family = matrix["tasks"][case_id]["family"]
                        if writeback_cli and family in {"partition", "organize"}:
                            package_root = writeback_package_root or (ROOT / "eval-data" / "assistant" / "writeback")
                            # Never carry model-claimed receipts/readbacks into a
                            # Workspace writeback artifact.  Only the CLI result
                            # below is authoritative for mutation evidence.
                            for field in (
                                "readback",
                                "readbacks",
                                "receipt",
                                "receipts",
                                "mutationReceipts",
                                "receiptAvailable",
                            ):
                                runner_metadata.pop(field, None)
                            try:
                                writeback = writeback_predictions(
                                    predictions,
                                    [public_record(record) for record in records],
                                    WritebackConfig(
                                        cli=writeback_cli,
                                        package_root=package_root,
                                        timeout=timeout_override or resources["timeoutMs"] / 1000,
                                        environment=environment,
                                    ),
                                    family=family,
                                    run_key=run_id,
                                )
                            except WritebackError as error:
                                status = "failed"
                                error_code = "WORKSPACE_WRITEBACK_FAILED"
                                reason = f"{error.code}: {error.reason}"
                                runner_metadata["writeback"] = {
                                    "status": "failed",
                                    "errorCode": error.code,
                                    "reason": error.reason,
                                    "mutationReceipts": error.receipts,
                                    "readback": error.readback,
                                }
                            else:
                                runner_metadata["writeback"] = {
                                    "status": "verified",
                                    "mutationReceipts": writeback["mutationReceipts"],
                                    "readback": writeback["readback"],
                                    "evidence": writeback["evidence"],
                                }
                                runner_metadata["mutationReceipts"] = writeback["mutationReceipts"]
                                runner_metadata["readback"] = writeback["readback"]
            except subprocess.TimeoutExpired:
                status = "failed"
                error_code = "EXTERNAL_RUNNER_TIMEOUT"
                reason = "runner exceeded timeoutMs"
            except (OSError, json.JSONDecodeError, MatrixError) as error:
                status = "failed"
                error_code = "EXTERNAL_RUNNER_RESPONSE_INVALID"
                reason = str(error)

    if status in {"pending_external_model", "pending_external_source", "not_assessed"}:
        prediction_payload: Any = {
            "schemaVersion": 1,
            "artifactType": "prediction",
            "status": status,
            "runId": run_id,
            "protocol": protocol,
            "caseId": case_id,
            "predictions": [],
            "errorCode": error_code,
            "reason": reason,
        }
        write_json(prediction_path, prediction_payload)
    elif status == "completed":
        prediction_path.parent.mkdir(parents=True, exist_ok=True)
        prediction_path.write_bytes(b"".join(canonical_json(item) + b"\n" for item in predictions))
    else:
        write_json(
            prediction_path,
            {
                "schemaVersion": 1,
                "artifactType": "prediction",
                "status": "failed",
                "runId": run_id,
                "protocol": protocol,
                "caseId": case_id,
                "predictions": [],
                "errorCode": error_code,
                "reason": reason,
            },
        )

    elapsed_ms = round((time.monotonic() - started) * 1000)
    metadata = {
        "schemaVersion": 1,
        "runId": run_id,
        "status": status,
        "protocol": protocol,
        "caseId": case_id,
        "model": matrix["model"]["requestedModel"],
        "prompt": f"{matrix['prompt']['id']}@{matrix['prompt']['version']}",
        "promptSha256": sha256_bytes(canonical_json(request["prompt"])),
        "budget": request["budget"],
        "resources": resources,
        "elapsedMs": elapsed_ms,
        "startedAt": started_at,
        "corpusSha256": corpus_hash,
        "holdoutManifestSha256": holdout_hash,
        "sourceManifestSha256": source_report["manifestSha256"],
        "sourceStatus": source_report["status"],
        "runnerConfigured": bool(runner),
        "predictionSource": "external_runner" if status == "completed" else None,
        "errorCode": error_code,
        "reason": reason,
        "runnerMetadata": runner_metadata,
    }
    write_json(metadata_path, metadata)
    readback = {
        "schemaVersion": 1,
        "runId": run_id,
        "status": status,
        "predictionReadback": {
            "path": prediction_path.name,
            "sha256": sha256_file(prediction_path),
            "scenarioCount": len(predictions),
            "scenarioIds": [item.get("scenarioId") for item in predictions],
        },
        "predictionSha256": sha256_file(prediction_path),
        "metadataSha256": sha256_file(metadata_path),
        "mutationReceipts": runner_metadata.get("mutationReceipts", []) if isinstance(runner_metadata.get("mutationReceipts", []), list) else [],
        "runnerMetadata": runner_metadata,
        "validation": validation,
        "structuredError": {"code": error_code, "reason": reason} if error_code or reason else None,
        "readbackAvailable": status == "completed",
    }
    write_json(readback_path, readback)
    receipt = {
        "schemaVersion": 1,
        "receiptType": "assistant-eval-run",
        "runId": run_id,
        "status": status,
        "protocol": protocol,
        "caseId": case_id,
        "requestSha256": request_hash,
        "artifactHashes": {
            "predictionFile": sha256_file(prediction_path),
            "runMetadata": sha256_file(metadata_path),
            "readback": sha256_file(readback_path),
        },
        "predictionSha256": sha256_file(prediction_path),
        "readbackSha256": sha256_file(readback_path),
        "mutationReceipts": runner_metadata.get("mutationReceipts", []) if isinstance(runner_metadata.get("mutationReceipts", []), list) else [],
        "runnerMetadata": runner_metadata,
        "errorCode": error_code,
        "reason": reason,
    }
    write_json(receipt_path, receipt)
    return {
        "runId": run_id,
        "protocol": protocol,
        "caseId": case_id,
        "status": status,
        "directory": str(run_dir),
        "reason": reason,
    }


def run_matrix(args: argparse.Namespace) -> dict[str, Any]:
    matrix = load_json(args.matrix)
    sources = load_json(args.sources)
    validate_model_matrix(matrix)
    source_report = validate_sources(sources, args.source_root)
    records, corpus_hash, holdout_hash = load_holdout(args.corpus, args.holdout)
    selected_protocols = [args.protocol] if args.protocol else list(PROTOCOLS)
    selected_cases = [args.case] if args.case else list(CASES)
    environment = dict(os.environ)
    results = []
    for protocol in selected_protocols:
        for case_id in selected_cases:
            case_records = select_case_records(records, matrix["tasks"][case_id])
            results.append(
                run_one(
                    matrix,
                    source_report,
                    case_records,
                    corpus_hash,
                    holdout_hash,
                    protocol,
                    case_id,
                    args.runner,
                    args.output_dir,
                    environment,
                    args.timeout,
                    getattr(args, "writeback_cli", None),
                    getattr(args, "writeback_package_root", None),
                )
            )
    statuses = {status: sum(result["status"] == status for result in results) for status in ALLOWED_STATUSES}
    return {
        "schemaVersion": 1,
        "matrixStatus": matrix["status"],
        "sourceStatus": source_report["status"],
        "sourceVerification": source_report,
        "corpusSha256": corpus_hash,
        "holdoutManifestSha256": holdout_hash,
        "runCount": len(results),
        "statuses": statuses,
        "runs": results,
    }


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser(description=__doc__)
    value.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    value.add_argument("--sources", type=Path, default=DEFAULT_SOURCES)
    value.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    value.add_argument("--holdout", type=Path, default=DEFAULT_HOLDOUT)
    value.add_argument("--source-root", type=Path, default=DEFAULT_SOURCE_ROOT)
    value.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT)
    value.add_argument("--runner", help="External runner command; receives one JSON request on stdin")
    value.add_argument("--protocol", choices=PROTOCOLS)
    value.add_argument("--case", choices=CASES)
    value.add_argument("--timeout", type=float, help="Override the matrix timeout for this invocation")
    value.add_argument(
        "--writeback-cli",
        help="Explicitly enable real Workspace writeback with a meetnote CLI command",
    )
    value.add_argument(
        "--writeback-package-root",
        type=Path,
        help="Temporary package root for writeback (defaults to eval-data/assistant/writeback)",
    )
    value.add_argument("--require-ready", action="store_true", help="Exit non-zero when any selected run is pending or failed")
    return value


def main() -> None:
    args = parser().parse_args()
    try:
        report = run_matrix(args)
    except (MatrixError, OSError) as error:
        parser().error(str(error))
    print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    if args.require_ready and any(status != "completed" for status in report["statuses"] for _ in range(report["statuses"][status])):
        raise SystemExit(2)


if __name__ == "__main__":
    main()
