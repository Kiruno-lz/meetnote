#!/usr/bin/env python3
"""Run an independent cross-scope semantic prediction and structural evaluation.

The fixture's gold labels are loaded by this evaluator only.  The request sent
to the external runner contains public document/block/segment scope and the
case contract, but never contains ``gold``, ``negativeCases``, or
``semanticRationale``.  A completed run proves that an independent model
returned a structurally valid prediction; it does not claim semantic quality
unless the emitted metrics support that claim.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import shlex
import subprocess
import sys
from pathlib import Path
from typing import Any

from semantic_schema import SemanticSchemaError, validate_semantic_fixture, validate_semantic_prediction


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_FIXTURE = Path(__file__).resolve().parent / "fixtures" / "cross-scope.json"
DEFAULT_RUNNER = [sys.executable, str(Path(__file__).resolve().parent / "runner.py")]
PROTOCOLS = {"openaiChatCompletions", "anthropicMessages", "openaiResponses"}
FAMILIES = {"partition", "extract", "none", "nested"}
FORBIDDEN_KEYS = {"gold", "negativecase", "negativecases", "semanticrationale", "rationale"}
RETRYABLE_CASE_ERRORS = {
    "PREDICTION_STRUCTURE_INVALID",
    "PREDICTION_COMPACT_INVALID",
    "PREDICTION_ALIAS_MISMATCH",
    "PREDICTION_PARENT_INVALID",
    "PREDICTION_COVERAGE_INVALID",
    "PREDICTION_CONTRACT_INVALID",
    "MODEL_OUTPUT_INVALID",
}


class CrossScopeError(ValueError):
    """A deterministic harness or external-runner failure."""

    def __init__(
        self,
        code: str,
        reason: str,
        *,
        status: str = "failed",
        details: list[dict[str, Any]] | None = None,
    ) -> None:
        super().__init__(reason)
        self.code = code
        self.reason = reason
        self.status = status
        self.details = details or []


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise CrossScopeError("FIXTURE_INVALID", f"cannot load fixture: {error}") from error
    if not isinstance(value, dict):
        raise CrossScopeError("FIXTURE_INVALID", "fixture root must be an object")
    return value


def _find_forbidden(value: Any, path: str = "$") -> str | None:
    if isinstance(value, dict):
        for key, child in value.items():
            if not isinstance(key, str):
                return f"{path} contains a non-string key"
            if key.lower() in FORBIDDEN_KEYS:
                return f"{path}.{key} is evaluator-only data"
            found = _find_forbidden(child, f"{path}.{key}")
            if found:
                return found
    elif isinstance(value, list):
        for index, child in enumerate(value):
            found = _find_forbidden(child, f"{path}[{index}]")
            if found:
                return found
    return None


def _public_scope(fixture: dict[str, Any], case: dict[str, Any]) -> dict[str, Any]:
    selected = set(case["inputBlockIds"])
    documents: list[dict[str, Any]] = []
    for document in fixture["documents"]:
        blocks: list[dict[str, Any]] = []
        for block in document["blocks"]:
            if block["blockId"] not in selected:
                continue
            blocks.append(
                {
                    "documentId": document["documentId"],
                    "blockId": block["blockId"],
                    "segments": [
                        {"segmentId": segment["segmentId"], "text": segment["text"]}
                        for segment in block["segments"]
                    ],
                }
            )
        if blocks:
            documents.append({"documentId": document["documentId"], "blocks": blocks})
    public_case = {
        "caseId": case["caseId"],
        "family": case["family"],
        "inputDocumentIds": list(case["inputDocumentIds"]),
        "inputBlockIds": list(case["inputBlockIds"]),
    }
    if case["family"] == "nested":
        public_case["selectedParentSegmentIds"] = [
            segment["segmentId"]
            for document in documents
            for block in document["blocks"]
            for segment in block["segments"]
        ]
    return {
        "documents": documents,
        "case": public_case,
    }


def _public_fixture(fixture: dict[str, Any], case: dict[str, Any]) -> dict[str, Any]:
    scope = _public_scope(fixture, case)
    return {
        "schemaVersion": 1,
        "fixtureType": "assistant-semantic-cross-scope-public",
        "documents": scope["documents"],
        "cases": [scope["case"]],
    }


def build_request(fixture: dict[str, Any], case: dict[str, Any], protocol: str) -> dict[str, Any]:
    if protocol not in PROTOCOLS:
        raise CrossScopeError("UNSUPPORTED_PROTOCOL", f"unsupported protocol: {protocol}", status="not_assessed")
    if case.get("family") not in FAMILIES:
        raise CrossScopeError("FIXTURE_INVALID", f"unsupported case family: {case.get('family')}")
    scope = _public_scope(fixture, case)
    segment_ids = [
        segment["segmentId"]
        for document in scope["documents"]
        for block in document["blocks"]
        for segment in block["segments"]
    ]
    required = ["scenarioId", "mode", "children", "uncovered_ranges", "uncoveredSpans"]
    if case["family"] == "nested":
        required.extend(["second_pass", "secondPass"])
    if case["family"] == "none":
        required.append("semanticRationale")
    public_case = scope["case"]
    public_case["segmentIds"] = segment_ids
    task = (
        "Return children as ordered, non-overlapping spans. Each span must carry "
        "documentId, blockId, and segmentIds; each child must carry childId and "
        "evidenceSegmentIds. Return uncoveredSpans and an identical alias "
        "uncovered_ranges. For nested, return both secondPass and second_pass "
        "aliases and preserve the selected parent range. For none, explain "
        "insufficient topic support using only public evidence."
    )
    if case["family"] == "none":
        task += (
            ' This is a none operation: set mode exactly to "none", return '
            "children exactly as [], uncoveredSpans exactly as [], and "
            "uncovered_ranges exactly as []; return only a model-generated "
            "semanticRationale explaining insufficient support from the supplied "
            "segments. Do not create a span or child."
        )
    if case["family"] == "nested":
        task += (
            ' This is a nested operation: set the top-level mode to "partition", '
            "create exactly one top-level child covering every selectedParentSegmentId, "
            "because the selected parent crosses blocks, represent that child with a "
            "spans array containing one explicit documentId/blockId/segmentIds span "
            "per input block; do not use a compact child documentId/blockId form, "
            "then select that emitted child by its childId in secondPass.parentChildId. "
            "Include the identical second_pass alias. Put second-pass children only "
            "inside that object, and require their children plus uncoveredSpans to "
            "equal the selected root child's complete segment range. Never choose a "
            "parent by an evaluator label or silently omit parentChildId."
        )
    request = {
        "schemaVersion": 1,
        "protocol": protocol,
        "caseId": case["caseId"],
        "taskFamily": case["family"],
        "requestedModel": "assistant-semantic-cross-scope-v1",
        "prompt": {
            "id": "assistant-semantic-cross-scope-v1",
            "version": 1,
            "system": (
                "Use only the supplied public document, block, and segment evidence. "
                "Do not infer evaluator labels or copy an oracle. Return one typed JSON "
                "prediction for the supplied case."
            ),
            "task": task,
        },
        "budget": {"maxToolRounds": 4, "maxOutputTokens": 2048},
        "resources": {
            "timeoutMs": 120000,
            "maxInputBytes": 4 * 1024 * 1024,
            "maxPredictionBytes": 2 * 1024 * 1024,
            "maxMutationReceipts": 16,
        },
        "holdout": {"goldIncluded": False, "scenarioCount": 1},
        "records": [
            {
                "scenarioId": case["caseId"],
                "taskIntent": f"Return an independent {case['family']} prediction for this public scope.",
                "record": {
                    "publicScope": scope,
                    "state": {"results": {"transcript": [{"id": item["segmentId"], "text": item["text"]} for document in scope["documents"] for block in document["blocks"] for item in block["segments"]]}},
                },
            }
        ],
        "semanticFixtures": [{"id": "cross-scope-public", "fixture": _public_fixture(fixture, case)}],
        "outputContract": {"required": required},
    }
    forbidden = _find_forbidden(request)
    if forbidden:
        raise CrossScopeError("PUBLIC_REQUEST_LEAK", forbidden)
    return request


def _run_external(command: list[str], request: dict[str, Any], timeout: float) -> dict[str, Any]:
    try:
        completed = subprocess.run(
            command,
            input=json.dumps(request, ensure_ascii=False),
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise CrossScopeError("EXTERNAL_RUNNER_TIMEOUT", "external runner exceeded timeout") from error
    except OSError as error:
        raise CrossScopeError("EXTERNAL_RUNNER_UNAVAILABLE", "cannot start external runner") from error
    if completed.returncode != 0:
        raise CrossScopeError("EXTERNAL_RUNNER_EXITED", f"external runner exited with {completed.returncode}")
    try:
        result = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise CrossScopeError("EXTERNAL_RUNNER_RESPONSE_INVALID", "external runner did not return JSON") from error
    if not isinstance(result, dict):
        raise CrossScopeError("EXTERNAL_RUNNER_RESPONSE_INVALID", "external runner response must be an object")
    status = result.get("status")
    if status in {"not_assessed", "pending_external_model", "pending_external_source"}:
        raise CrossScopeError("NOT_ASSESSED", str(result.get("reason", "external model evidence is unavailable")), status="not_assessed")
    if status != "completed":
        raise CrossScopeError(str(result.get("errorCode", "EXTERNAL_RUNNER_FAILED")), str(result.get("reason", "external runner failed")))
    if not isinstance(result.get("predictions"), list):
        raise CrossScopeError("EXTERNAL_RUNNER_RESPONSE_INVALID", "completed response has no predictions")
    return result


def _segment_order(fixture: dict[str, Any], case: dict[str, Any]) -> list[str]:
    blocks = {block["blockId"]: block for document in fixture["documents"] for block in document["blocks"]}
    return [segment["segmentId"] for block_id in case["inputBlockIds"] for segment in blocks[block_id]["segments"]]


def _boundaries(prediction: dict[str, Any], order: list[str], *, second_pass: bool = False) -> set[int]:
    node: dict[str, Any] = prediction
    if second_pass:
        node = prediction.get("secondPass", {})
    children = node.get("children", []) if isinstance(node, dict) else []
    index = {segment_id: position for position, segment_id in enumerate(order)}
    boundaries: set[int] = set()
    for child in children:
        positions = [index[segment_id] for span in child.get("spans", []) for segment_id in span.get("segmentIds", []) if segment_id in index]
        if positions:
            end = max(positions)
            if end < len(order) - 1:
                boundaries.add(end)
    return boundaries


def _f1(expected: set[int], actual: set[int]) -> dict[str, float]:
    true_positive = len(expected & actual)
    precision = true_positive / len(actual) if actual else (1.0 if not expected else 0.0)
    recall = true_positive / len(expected) if expected else (1.0 if not actual else 0.0)
    f1 = 2 * precision * recall / (precision + recall) if precision + recall else 0.0
    return {"precision": precision, "recall": recall, "f1": f1}


def _covered(prediction: dict[str, Any]) -> set[str]:
    return {segment_id for child in prediction.get("children", []) for span in child.get("spans", []) for segment_id in span.get("segmentIds", [])}


def _public_block_ownership(fixture: dict[str, Any], case: dict[str, Any]) -> dict[str, tuple[str, set[str]]]:
    selected = set(case["inputBlockIds"])
    ownership: dict[str, tuple[str, set[str]]] = {}
    for document in fixture["documents"]:
        for block in document["blocks"]:
            if block["blockId"] in selected:
                ownership[block["blockId"]] = (
                    document["documentId"],
                    {segment["segmentId"] for segment in block["segments"]},
                )
    return ownership


def _normalize_compact_spans(
    node: dict[str, Any], ownership: dict[str, tuple[str, set[str]]], label: str
) -> None:
    """Normalize one compact child only when its ownership is explicit.

    This deliberately does not infer a block from a segment ID or from a
    range.  A compact child must carry one documentId, one blockId, and a
    non-empty segmentIds array whose IDs all belong to that exact public block.
    """
    children = node.get("children")
    if not isinstance(children, list):
        return
    for index, child in enumerate(children):
        if not isinstance(child, dict) or "spans" in child:
            continue
        document_id = child.get("documentId")
        block_id = child.get("blockId")
        segment_ids = child.get("segmentIds")
        if not isinstance(document_id, str) or not document_id:
            raise CrossScopeError(
                "PREDICTION_COMPACT_INVALID",
                f"{label}.children[{index}] compact child requires documentId",
            )
        if not isinstance(block_id, str) or not block_id:
            raise CrossScopeError(
                "PREDICTION_COMPACT_INVALID",
                f"{label}.children[{index}] compact child requires blockId",
            )
        if not isinstance(segment_ids, list) or not segment_ids or not all(
            isinstance(segment_id, str) and segment_id for segment_id in segment_ids
        ):
            raise CrossScopeError(
                "PREDICTION_COMPACT_INVALID",
                f"{label}.children[{index}] compact child requires non-empty segmentIds",
            )
        expected = ownership.get(block_id)
        if expected is None or expected[0] != document_id or not set(segment_ids) <= expected[1]:
            raise CrossScopeError(
                "PREDICTION_COMPACT_INVALID",
                f"{label}.children[{index}] compact child escapes its public document/block",
            )
        child["spans"] = [
            {"documentId": document_id, "blockId": block_id, "segmentIds": list(segment_ids)}
        ]
        if "evidenceSegmentIds" not in child and isinstance(child.get("evidence_segment_ids"), list):
            child["evidenceSegmentIds"] = list(child["evidence_segment_ids"])

    uncovered = node.get("uncoveredSpans")
    if not isinstance(uncovered, list):
        return
    for index, span in enumerate(uncovered):
        if not isinstance(span, dict) or "segmentIds" not in span or "spans" in span:
            continue
        document_id = span.get("documentId")
        block_id = span.get("blockId")
        segment_ids = span.get("segmentIds")
        expected = ownership.get(block_id) if isinstance(block_id, str) else None
        if (
            not isinstance(document_id, str)
            or not isinstance(block_id, str)
            or not isinstance(segment_ids, list)
            or not segment_ids
            or not all(isinstance(segment_id, str) and segment_id for segment_id in segment_ids)
            or expected is None
            or expected[0] != document_id
            or not set(segment_ids) <= expected[1]
        ):
            raise CrossScopeError(
                "PREDICTION_COMPACT_INVALID",
                f"{label}.uncoveredSpans[{index}] compact span has invalid public ownership",
            )


def _normalize_prediction(
    fixture: dict[str, Any], case: dict[str, Any], prediction: dict[str, Any]
) -> dict[str, Any]:
    value = copy.deepcopy(prediction)
    if "caseId" not in value and isinstance(value.get("scenarioId"), str):
        value["caseId"] = value["scenarioId"]
    if "uncoveredSpans" not in value and isinstance(value.get("uncovered_ranges"), list):
        value["uncoveredSpans"] = value["uncovered_ranges"]
    if "secondPass" not in value and isinstance(value.get("second_pass"), dict):
        nested = copy.deepcopy(value["second_pass"])
        if "parentChildId" not in nested and isinstance(nested.get("parent_child_index"), int):
            children = value.get("children", [])
            index = nested["parent_child_index"]
            if 0 <= index < len(children):
                nested["parentChildId"] = children[index].get("childId")
        if "parentChildId" not in nested:
            children = value.get("children", [])
            if isinstance(children, list) and len(children) == 1 and isinstance(children[0], dict):
                parent_id = children[0].get("childId")
                if isinstance(parent_id, str) and parent_id:
                    nested["parentChildId"] = parent_id
        nested.setdefault("uncoveredSpans", nested.get("uncovered_ranges", []))
        value["secondPass"] = nested
    ownership = _public_block_ownership(fixture, case)
    _normalize_compact_spans(value, ownership, "prediction")
    if isinstance(value.get("secondPass"), dict):
        _normalize_compact_spans(value["secondPass"], ownership, "prediction.secondPass")
    return value


def evaluate_case(fixture: dict[str, Any], case: dict[str, Any], prediction: dict[str, Any]) -> dict[str, Any]:
    normalized = _normalize_prediction(fixture, case, prediction)
    try:
        validate_semantic_prediction(fixture, normalized)
    except SemanticSchemaError as error:
        raise CrossScopeError("PREDICTION_STRUCTURE_INVALID", str(error)) from error
    gold = case["gold"]
    order = _segment_order(fixture, case)
    gold_prediction = {"caseId": case["caseId"], **gold}
    boundary = _f1(_boundaries(gold_prediction, order), _boundaries(normalized, order))
    gold_covered = _covered(gold_prediction)
    predicted_covered = _covered(normalized)
    coverage = len(gold_covered & predicted_covered) / len(gold_covered) if gold_covered else (1.0 if not predicted_covered else 0.0)
    result: dict[str, Any] = {
        "caseId": case["caseId"],
        "family": case["family"],
        "structureValid": True,
        "modeMatch": normalized.get("mode") == gold.get("mode"),
        "coverage": coverage,
        "boundary": boundary,
        "predictionSha256": hashlib.sha256(json.dumps(prediction, ensure_ascii=False, sort_keys=True).encode("utf-8")).hexdigest(),
    }
    if case["family"] == "nested":
        result["secondPassBoundary"] = _f1(
            _boundaries(gold_prediction, order, second_pass=True),
            _boundaries(normalized, order, second_pass=True),
        )
    return result


def run_evaluation(fixture: dict[str, Any], command: list[str], protocol: str) -> dict[str, Any]:
    try:
        validate_semantic_fixture(fixture)
    except SemanticSchemaError as error:
        raise CrossScopeError("FIXTURE_INVALID", str(error)) from error
    cases = {case["caseId"]: case for case in fixture["cases"]}
    required = {"cross-block-partition", "cross-document-extract", "single-topic-none", "cross-block-second-pass"}
    if set(cases) != required:
        raise CrossScopeError("FIXTURE_INVALID", "cross-scope fixture case coverage is incomplete")
    reports: list[dict[str, Any]] = []
    failures: list[dict[str, Any]] = []
    for case in cases.values():
        request = build_request(fixture, case, protocol)
        last_error: CrossScopeError | None = None
        for attempt in range(1, 4):
            try:
                request_for_attempt = request
                if attempt > 1 and last_error is not None:
                    request_for_attempt = copy.deepcopy(request)
                    prompt = dict(request_for_attempt.get("prompt", {}))
                    prompt["task"] = (
                        f"{prompt.get('task', '')}\n\nREPAIR REQUIRED: {last_error.reason}. "
                        "Return the complete public contract and do not omit any required field."
                    )
                    request_for_attempt["prompt"] = prompt
                result = _run_external(command, request_for_attempt, timeout=120.0)
                predictions = result["predictions"]
                matching = [
                    item
                    for item in predictions
                    if isinstance(item, dict) and item.get("scenarioId") == case["caseId"]
                ]
                if len(matching) != 1:
                    raise CrossScopeError(
                        "PREDICTION_COVERAGE_INVALID",
                        f"expected one prediction for {case['caseId']}",
                    )
                report = evaluate_case(fixture, case, matching[0])
                report["status"] = "completed"
                report["attempt"] = attempt
                reports.append(report)
                last_error = None
                break
            except CrossScopeError as error:
                last_error = error
                if error.status == "not_assessed" or error.code not in RETRYABLE_CASE_ERRORS:
                    break
        if last_error is not None:
            failure = {
                "caseId": case["caseId"],
                "family": case["family"],
                "status": last_error.status,
                "errorCode": last_error.code,
                "reason": last_error.reason,
                "attempts": attempt,
            }
            failures.append(failure)
            reports.append(failure)
    if failures:
        status = "not_assessed" if all(item["status"] == "not_assessed" for item in failures) else "failed"
        raise CrossScopeError(
            "CROSS_SCOPE_CASES_FAILED",
            (
                f"{len(failures)} of {len(cases)} cross-scope cases failed; "
                f"first={failures[0]['caseId']}:{failures[0]['errorCode']}:{failures[0]['reason']}"
            ),
            status=status,
            details=reports,
        )
    boundary_f1 = sum(item["boundary"]["f1"] for item in reports) / len(reports)
    return {
        "schemaVersion": 1,
        "status": "completed",
        "qualityStatus": "assessed_structural",
        "predictionSource": "external_runner",
        "protocol": protocol,
        "caseCount": len(reports),
        "structuralValidCount": sum(item["structureValid"] for item in reports),
        "boundaryF1": boundary_f1,
        "cases": reports,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runner", default=" ".join(DEFAULT_RUNNER), help="external runner command")
    parser.add_argument("--fixture", type=Path, default=DEFAULT_FIXTURE)
    parser.add_argument("--protocol", choices=sorted(PROTOCOLS), default="openaiResponses")
    args = parser.parse_args(argv)
    try:
        report = run_evaluation(load_json(args.fixture), shlex.split(args.runner), args.protocol)
    except CrossScopeError as error:
        report = {
            "schemaVersion": 1,
            "status": error.status,
            "errorCode": error.code,
            "reason": error.reason,
            "predictionSource": "external_runner",
        }
        if error.details:
            report["caseCount"] = len(error.details)
            report["failedCaseCount"] = sum(item.get("status") != "completed" for item in error.details)
            report["cases"] = error.details
    print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
