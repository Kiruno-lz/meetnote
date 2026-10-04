#!/usr/bin/env python3
"""Evaluate an external Assistant runner on converted QMSum or AMI records.

The model receives only a public text projection.  Gold ranges, topic
hierarchy, query answers, and annotation rationale remain in the evaluator
process and are used only after the runner returns predictions.  Missing
external configuration is an explicit ``not_assessed`` result.
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
from pathlib import Path
from typing import Any, Iterable, Mapping

from evaluate import compare_with_baselines, evaluate_records, read_jsonl


ROOT = Path(__file__).resolve().parents[2]
SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_DATA = ROOT / "eval-data/assistant/data/converted"
DEFAULT_OUTPUT = ROOT / "eval-data/assistant/data/external"
DEFAULT_RUNNER = f"{shlex.join([sys.executable, str(SCRIPT_DIR / 'runner.py')])}"
DATASET_MAX_OUTPUT_TOKENS = 4096
BOUNDARY_WINDOW_SEGMENTS = 64
BOUNDARY_WINDOW_OVERLAP = 8
DATASET_FILES = {
    "qmsum": DEFAULT_DATA / "qmsum.jsonl",
    "ami": DEFAULT_DATA / "ami.jsonl",
}
DATASET_TASKS = {
    "qmsum": {
        "family": "extract",
        "mode": "extract",
        "task": (
            "Extract only the requested topic occurrences as continuous, ordered, "
            "non-overlapping ranges. Return every complementary range in "
            "uncovered_ranges; do not turn an extract task into a full partition. "
            "If the topic is one coherent occurrence, keep it as one child."
        ),
    },
    "ami": {
        "family": "partition",
        "mode": "partition",
        "task": (
            "Partition the transcript into evidence-backed topic ranges. Return "
            "continuous, ordered, non-overlapping ranges with complete coverage. "
            "Create a boundary when the discussed topic object, decision, task, "
            "or work phase changes, even when the same speaker continues and the "
            "turns are adjacent; do not merge distinct topics for conversational "
            "continuity alone. "
            "Use only observable agenda and purpose transitions as cues: a new "
            "agenda item, an explicit chair or facilitator transition, a new "
            "presenter or demonstration stage, a task or action-item assignment, "
            "and a meeting closing or wrap-up. Treat these as candidate boundaries "
            "only when the surrounding transcript supports the transition. "
            "Before returning JSON, scan every adjacent turn for a change in topic "
            "object, decision, task, or work phase and revise the ranges if a "
            "semantic or agenda transition was missed; ensure the final JSON contains "
            "the resulting segment boundaries. "
            "Do not add a boundary solely to reach a target number of children; "
            "a single coherent topic must remain one child. Use transcript evidence "
            "only; never infer boundaries or child count from a target number, record "
            "ID, title, gold label, or annotation metadata."
        ),
    },
}
PROTOCOLS = {
    "openaiChatCompletions",
    "anthropicMessages",
    "openaiResponses",
}

ANNOTATION_KEYS = frozenset(
    {
        "gold",
        "negativecase",
        "negativecases",
        "semanticrationale",
        "rationale",
        "topichierarchy",
        "queries",
        "answer",
        "relevant_text_span",
        "relevanttextspan",
    }
)
OUTPUT_ANNOTATION_KEYS = frozenset(
    {
        "gold",
        "negativecase",
        "negativecases",
        "queries",
        "answer",
        "relevant_text_span",
        "relevanttextspan",
        "topichierarchy",
    }
)


class DatasetEvalError(ValueError):
    """Raised for invalid data, runner output, or evaluation evidence."""


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def _contains_annotation_key(value: Any, path: str = "$") -> str | None:
    if isinstance(value, dict):
        for key, child in value.items():
            if isinstance(key, str) and key.lower() in ANNOTATION_KEYS:
                return f"{path}.{key}"
            found = _contains_annotation_key(child, f"{path}.{key}")
            if found:
                return found
    elif isinstance(value, list):
        for index, child in enumerate(value):
            found = _contains_annotation_key(child, f"{path}[{index}]")
            if found:
                return found
    return None


def _contains_output_annotation_key(value: Any, path: str = "$") -> str | None:
    """Reject copied annotation fields while allowing model rationale."""
    if isinstance(value, dict):
        for key, child in value.items():
            if isinstance(key, str) and key.lower() in OUTPUT_ANNOTATION_KEYS:
                return f"{path}.{key}"
            found = _contains_output_annotation_key(child, f"{path}.{key}")
            if found:
                return found
    elif isinstance(value, list):
        for index, child in enumerate(value):
            found = _contains_output_annotation_key(child, f"{path}[{index}]")
            if found:
                return found
    return None


def public_record(record: Mapping[str, Any]) -> dict[str, Any]:
    """Build the only record projection permitted in a model request."""
    segments = record.get("segments")
    if not isinstance(segments, list) or not segments:
        raise DatasetEvalError("record must contain a non-empty segments array")
    public_segments = []
    for index, segment in enumerate(segments):
        if not isinstance(segment, dict):
            raise DatasetEvalError(f"segment[{index}] must be an object")
        segment_id = segment.get("id")
        text = segment.get("text")
        if not isinstance(segment_id, str) or not segment_id:
            raise DatasetEvalError(f"segment[{index}] has no stable id")
        if not isinstance(text, str):
            raise DatasetEvalError(f"segment[{index}] has no text")
        public_segments.append(
            {
                "id": segment_id,
                "startMs": segment.get("startMs"),
                "endMs": segment.get("endMs"),
                "speaker": segment.get("speaker"),
                "text": text,
            }
        )
    result = {
        "schemaVersion": record.get("schemaVersion", 1),
        "scenarioId": record.get("scenarioId"),
        "segments": public_segments,
    }
    if not isinstance(result["scenarioId"], str) or not result["scenarioId"]:
        raise DatasetEvalError("record has no scenarioId")
    forbidden = _contains_annotation_key(result)
    if forbidden:
        raise DatasetEvalError(f"public record contains evaluator-only field: {forbidden}")
    return result


def load_records(path: Path, limit: int | None = None, single_topic_only: bool = False) -> list[dict[str, Any]]:
    records = read_jsonl(path)
    selected: list[dict[str, Any]] = []
    for record in records:
        gold = record.get("gold")
        if not isinstance(gold, dict):
            raise DatasetEvalError(f"{path}: record has no evaluator gold")
        if single_topic_only and len(gold.get("children", [])) != 1:
            continue
        selected.append(record)
        if limit is not None and len(selected) >= limit:
            break
    if not selected:
        criterion = "single-topic" if single_topic_only else "records"
        raise DatasetEvalError(f"{path}: no {criterion} selected")
    return selected


def build_request(
    dataset: str,
    protocol: str,
    records: list[dict[str, Any]],
    *,
    single_topic_only: bool,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    if protocol not in PROTOCOLS:
        raise DatasetEvalError(f"unsupported protocol: {protocol}")
    task = DATASET_TASKS.get(dataset)
    if task is None:
        raise DatasetEvalError(f"unsupported dataset: {dataset}")
    public_records = [public_record(record) for record in records]
    request = {
        "schemaVersion": 1,
        "protocol": protocol,
        "caseId": f"{dataset}-converted-{task['family']}",
        "taskFamily": task["family"],
        "requestedModel": "assistant-eval-external-v1",
        "prompt": {
            "id": "assistant-dataset-semantic-eval-v1",
            "version": 1,
            "system": (
                "Use only the supplied transcript segments as evidence. "
                "Do not infer labels from record IDs or metadata."
            ),
            "task": task["task"],
        },
        "budget": {"maxToolRounds": 1, "maxOutputTokens": DATASET_MAX_OUTPUT_TOKENS},
        "resources": {
            "maxToolRounds": 1,
            "maxOutputTokens": DATASET_MAX_OUTPUT_TOKENS,
            "timeoutMs": 120_000,
            "maxInputBytes": 4 * 1024 * 1024,
            "maxPredictionBytes": 2 * 1024 * 1024,
            "maxReadbackBytes": 512 * 1024,
            "maxMutationReceipts": 16,
            "concurrency": 1,
        },
        "holdout": {
            "dataset": dataset,
            "sampleCount": len(public_records),
            "singleTopicOnly": single_topic_only,
            "goldIncluded": False,
        },
        "records": public_records,
        "outputContract": {
            "required": ["scenarioId", "mode", "children", "uncovered_ranges"],
            "expectedMode": task["mode"],
        },
    }
    forbidden = _contains_annotation_key(request)
    if forbidden:
        raise DatasetEvalError(f"model request contains evaluator-only field: {forbidden}")
    return request, public_records


def build_boundary_request(
    dataset: str,
    protocol: str,
    records: list[dict[str, Any]],
    *,
    window: dict[str, Any] | None = None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Build a public adjacent-transition request for the boundary-first experiment."""
    if dataset != "ami":
        raise DatasetEvalError("boundary-first experiment is currently defined for AMI")
    if protocol not in PROTOCOLS:
        raise DatasetEvalError(f"unsupported protocol: {protocol}")
    public_records = [public_record(record) for record in records]
    request = {
        "schemaVersion": 1,
        "protocol": protocol,
        "caseId": "ami-converted-boundary-first",
        "taskFamily": "boundary",
        "requestedModel": "assistant-eval-external-v1",
        "prompt": {
            "id": "assistant-ami-boundary-first-v1",
            "version": 1,
            "system": (
                "Use only the supplied public transcript segments, speaker labels, "
                "and timing. Do not infer labels from IDs or metadata."
            ),
            "task": (
                "For every adjacent pair of transcript segments, return exactly one "
                "transition decision keyed by the preceding segment ID. Set boundary "
                "true only when the evidence shows a change in topic object, decision, "
                "task, work phase, agenda item, presenter/demo stage, action-item "
                "assignment, or meeting closing. Same-speaker continuity is not a "
                "reason to merge distinct topics. Do not use target child count, titles, "
                "gold labels, annotation metadata, or record IDs as evidence."
            ),
        },
        "budget": {"maxToolRounds": 1, "maxOutputTokens": DATASET_MAX_OUTPUT_TOKENS},
        "resources": {
            "maxToolRounds": 1,
            "maxOutputTokens": DATASET_MAX_OUTPUT_TOKENS,
            "timeoutMs": 120_000,
            "maxInputBytes": 4 * 1024 * 1024,
            "maxPredictionBytes": 2 * 1024 * 1024,
            "maxReadbackBytes": 512 * 1024,
            "maxMutationReceipts": 16,
            "concurrency": 1,
        },
        "holdout": {
            "dataset": "ami",
            "sampleCount": len(public_records),
            "goldIncluded": False,
            "window": window,
        },
        "records": public_records,
        "outputContract": {
            "required": ["scenarioId", "transitions"],
            "transitionRequired": ["after_segment_id", "boundary"],
        },
    }
    forbidden = _contains_annotation_key(request)
    if forbidden:
        raise DatasetEvalError(f"boundary request contains evaluator-only field: {forbidden}")
    return request, public_records


def split_boundary_windows(
    record: dict[str, Any],
    window_size: int = BOUNDARY_WINDOW_SEGMENTS,
    overlap: int = BOUNDARY_WINDOW_OVERLAP,
) -> list[tuple[int, int]]:
    """Return contiguous [start,end) windows whose overlap preserves every boundary."""
    segments = record.get("segments")
    if not isinstance(segments, list) or not segments:
        raise DatasetEvalError("boundary window record must contain segments")
    if window_size < 2 or overlap < 0 or overlap >= window_size - 1:
        raise DatasetEvalError("boundary window size/overlap is invalid")
    if len(segments) <= window_size:
        return [(0, len(segments))]
    windows: list[tuple[int, int]] = []
    start = 0
    while start < len(segments):
        end = min(len(segments), start + window_size)
        windows.append((start, end))
        if end == len(segments):
            break
        start = end - overlap
    return windows


def validate_boundary_predictions(
    records: list[dict[str, Any]], predictions: list[dict[str, Any]]
) -> None:
    expected_ids = {record["scenarioId"] for record in records}
    actual_ids = {prediction.get("scenarioId") for prediction in predictions}
    if actual_ids != expected_ids or len(actual_ids) != len(predictions):
        raise DatasetEvalError("boundary predictions do not cover selected records exactly once")
    by_id = {prediction["scenarioId"]: prediction for prediction in predictions}
    for record in records:
        prediction = by_id[record["scenarioId"]]
        segment_ids = [segment["id"] for segment in record["segments"]]
        expected_after = set(segment_ids[:-1])
        transitions = prediction.get("transitions")
        if not isinstance(transitions, list):
            raise DatasetEvalError("boundary prediction transitions must be an array")
        actual_after = [item.get("after_segment_id") for item in transitions]
        if set(actual_after) != expected_after or len(actual_after) != len(set(actual_after)):
            raise DatasetEvalError(
                f"boundary transitions for {record['scenarioId']} must cover every adjacent pair exactly once"
            )
        for item in transitions:
            if not isinstance(item.get("boundary"), bool):
                raise DatasetEvalError("boundary decision must be boolean")


def assemble_boundary_predictions(
    records: list[dict[str, Any]], predictions: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    """Assemble model transition decisions into deterministic partition ranges."""
    validate_boundary_predictions(records, predictions)
    by_id = {prediction["scenarioId"]: prediction for prediction in predictions}
    assembled: list[dict[str, Any]] = []
    for record in records:
        segment_ids = [segment["id"] for segment in record["segments"]]
        by_after = {
            item["after_segment_id"]: item["boundary"]
            for item in by_id[record["scenarioId"]]["transitions"]
        }
        start = 0
        children: list[dict[str, Any]] = []
        for index, segment_id in enumerate(segment_ids[:-1]):
            if by_after[segment_id]:
                children.append(
                    {
                        "start_segment_id": segment_ids[start],
                        "end_segment_id": segment_ids[index],
                        "evidence_segment_ids": [segment_ids[start], segment_ids[index]],
                    }
                )
                start = index + 1
        children.append(
            {
                "start_segment_id": segment_ids[start],
                "end_segment_id": segment_ids[-1],
                "evidence_segment_ids": [segment_ids[start], segment_ids[-1]],
            }
        )
        assembled.append(
            {
                "scenarioId": record["scenarioId"],
                "mode": "partition",
                "children": children,
                "uncovered_ranges": [],
            }
        )
    return assembled


def validate_dataset_predictions(
    dataset: str, records: list[dict[str, Any]], predictions: list[dict[str, Any]]
) -> None:
    """Reject a response that changes the dataset's semantic operation."""
    task = DATASET_TASKS.get(dataset)
    if task is None:
        raise DatasetEvalError(f"unsupported dataset: {dataset}")
    expected_ids = {record["scenarioId"] for record in records}
    actual_ids = {prediction.get("scenarioId") for prediction in predictions}
    if actual_ids != expected_ids or len(actual_ids) != len(predictions):
        raise DatasetEvalError("external predictions do not cover selected records exactly once")
    for prediction in predictions:
        if prediction.get("mode") != task["mode"]:
            raise DatasetEvalError(
                f"{dataset} prediction {prediction.get('scenarioId')} must use mode {task['mode']}"
            )
        if not isinstance(prediction.get("uncovered_ranges"), list):
            raise DatasetEvalError("prediction uncovered_ranges must be an array")


def _runner_status(value: Any) -> tuple[str, list[dict[str, Any]], dict[str, Any], str | None]:
    if not isinstance(value, dict):
        raise DatasetEvalError("runner output must be a JSON object")
    status = value.get("status")
    predictions = value.get("predictions", [])
    metadata = value.get("metadata") if isinstance(value.get("metadata"), dict) else {}
    reason = value.get("reason")
    if status not in {"completed", "failed", "not_assessed", "pending_external_model"}:
        raise DatasetEvalError(f"runner returned unsupported status: {status!r}")
    if not isinstance(predictions, list) or not all(isinstance(item, dict) for item in predictions):
        raise DatasetEvalError("runner predictions must be an array of objects")
    if status == "completed":
        forbidden = _contains_output_annotation_key(predictions)
        if forbidden:
            raise DatasetEvalError(f"runner prediction contains evaluator-only field: {forbidden}")
    return status, predictions, metadata, reason if isinstance(reason, str) else None


def _run_external(
    request: dict[str, Any], runner: str, environment: Mapping[str, str], timeout: float
) -> tuple[str, list[dict[str, Any]], dict[str, Any], str | None, str]:
    command = shlex.split(runner)
    if not command:
        raise DatasetEvalError("external runner command is empty")
    encoded_request = json.dumps(request, ensure_ascii=False, sort_keys=True).encode("utf-8")
    started = time.monotonic()
    completed = subprocess.run(
        command,
        input=encoded_request.decode("utf-8"),
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
        env=dict(environment),
    )
    elapsed_ms = round((time.monotonic() - started) * 1000)
    if completed.returncode != 0:
        raise DatasetEvalError(f"external runner exited with code {completed.returncode}")
    if completed.stderr:
        raise DatasetEvalError("external runner wrote to stderr")
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise DatasetEvalError("external runner did not return JSON") from error
    status, predictions, metadata, reason = _runner_status(value)
    metadata = copy.deepcopy(metadata)
    metadata.setdefault("elapsedMs", elapsed_ms)
    return status, predictions, metadata, reason, sha256_bytes(completed.stdout.encode("utf-8"))


def _single_topic_check(
    dataset: str, records: Iterable[dict[str, Any]], predictions: list[dict[str, Any]]
) -> dict[str, Any]:
    expected_mode = DATASET_TASKS[dataset]["mode"]
    by_id = {item.get("scenarioId"): item for item in predictions}
    checks = []
    for record in records:
        gold = record["gold"]
        if len(gold.get("children", [])) != 1:
            continue
        prediction = by_id.get(record["scenarioId"])
        preserved = (
            isinstance(prediction, dict)
            and prediction.get("mode") == expected_mode
            and len(prediction.get("children", [])) == 1
        )
        checks.append({"scenarioId": record["scenarioId"], "preserved": preserved})
    return {
        "sampleCount": len(checks),
        "passed": bool(checks) and all(item["preserved"] for item in checks),
        "checks": checks,
    }


def quality_thresholds(single_topic_only: bool) -> dict[str, float]:
    """Use a no-internal-boundary threshold for the dedicated single-topic run."""
    return {
        "structuralValidityRate": 1.0,
        "evidenceValidityRate": 1.0,
        "hierarchyValidityRate": 1.0,
        "boundaryF1": 0.0 if single_topic_only else 0.8,
        "Pk": 0.2,
        "WindowDiff": 0.2,
    }


def run_dataset(
    dataset: str,
    input_path: Path,
    *,
    protocol: str,
    runner: str,
    limit: int,
    single_topic_only: bool,
    environment: Mapping[str, str],
    timeout: float,
    output_dir: Path | None = None,
) -> dict[str, Any]:
    records = load_records(input_path, limit=limit, single_topic_only=single_topic_only)
    request, public_records = build_request(
        dataset, protocol, records, single_topic_only=single_topic_only
    )
    request_bytes = json.dumps(request, ensure_ascii=False, sort_keys=True).encode("utf-8")
    base = {
        "schemaVersion": 1,
        "dataset": dataset,
        "protocol": protocol,
        "status": "not_assessed",
        "predictionSource": "external_runner",
        "sampleCount": len(records),
        "singleTopicOnly": single_topic_only,
        "goldUsedForEvaluationOnly": True,
        "annotationInputToModel": False,
        "requestSha256": sha256_bytes(request_bytes),
        "sourceFile": str(input_path),
        "sourceSha256": sha256_file(input_path),
    }
    try:
        status, predictions, metadata, reason, runner_output_sha256 = _run_external(
            request, runner, environment, timeout
        )
    except (DatasetEvalError, OSError, subprocess.TimeoutExpired) as error:
        base.update({"status": "failed", "reason": str(error)})
        predictions = []
        metadata = {}
        runner_output_sha256 = None
    else:
        base.update({"status": status, "reason": reason, "runnerMetadata": metadata})
        base["runnerOutputSha256"] = runner_output_sha256
    if base["status"] == "completed":
        try:
            validate_dataset_predictions(dataset, records, predictions)
            structural = evaluate_records(records, predictions)
            comparison = compare_with_baselines(records, predictions)
        except (KeyError, TypeError, ValueError) as error:
            base.update({"status": "failed", "reason": f"evaluation failed: {error}"})
        else:
            single_topic = _single_topic_check(dataset, records, predictions)
            thresholds = quality_thresholds(single_topic_only)
            passed = (
                structural["structuralValidityRate"] >= thresholds["structuralValidityRate"]
                and structural["evidenceValidityRate"] >= thresholds["evidenceValidityRate"]
                and structural["hierarchyValidityRate"] >= thresholds["hierarchyValidityRate"]
                and structural["boundaryF1"] >= thresholds["boundaryF1"]
                and structural["Pk"] <= thresholds["Pk"]
                and structural["WindowDiff"] <= thresholds["WindowDiff"]
                and (not single_topic_only or single_topic["passed"])
            )
            base.update(
                {
                    "qualityStatus": "passed" if passed else "failed",
                    "evaluation": structural,
                    "baselineComparison": comparison,
                    "singleTopicCheck": single_topic,
                    "thresholds": thresholds,
                }
            )
    prediction_bytes = b"".join(
        json.dumps(item, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8") + b"\n"
        for item in predictions
    )
    base["predictionSha256"] = sha256_bytes(prediction_bytes)
    if output_dir is not None:
        output_dir.mkdir(parents=True, exist_ok=True)
        (output_dir / "prediction.jsonl").write_bytes(prediction_bytes)
        (output_dir / "metadata.json").write_text(
            json.dumps(
                {
                    "dataset": dataset,
                    "protocol": protocol,
                    "sampleCount": len(public_records),
                    "requestSha256": base["requestSha256"],
                    "runnerMetadata": base.get("runnerMetadata", {}),
                    "status": base["status"],
                    "predictionSource": "external_runner",
                },
                ensure_ascii=False,
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        (output_dir / "report.json").write_text(
            json.dumps(base, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    return base


def run_boundary_first_dataset(
    input_path: Path,
    *,
    protocol: str,
    runner: str,
    limit: int,
    environment: Mapping[str, str],
    timeout: float,
    output_dir: Path | None = None,
) -> dict[str, Any]:
    """Run bounded boundary detection, then assemble ranges without evaluator gold."""
    records = load_records(input_path, limit=limit, single_topic_only=False)
    window_requests: list[tuple[dict[str, Any], dict[str, Any], dict[str, Any]]] = []
    for record in records:
        for start, end in split_boundary_windows(record):
            chunk = copy.deepcopy(record)
            chunk["segments"] = record["segments"][start:end]
            window = {
                "scenarioId": record["scenarioId"],
                "startSegmentIndex": start,
                "endSegmentIndexExclusive": end,
                "overlapSegments": BOUNDARY_WINDOW_OVERLAP,
            }
            request, _ = build_boundary_request("ami", protocol, [chunk], window=window)
            window_requests.append((request, window, chunk))
    request_hashes = [sha256_bytes(json.dumps(request, ensure_ascii=False, sort_keys=True).encode("utf-8")) for request, _, _ in window_requests]
    base: dict[str, Any] = {
        "schemaVersion": 1,
        "dataset": "ami",
        "protocol": protocol,
        "boundaryFirst": True,
        "algorithm": "boundary_first_deterministic_assembly",
        "status": "not_assessed",
        "predictionSource": "external_runner",
        "sampleCount": len(records),
        "goldUsedForEvaluationOnly": True,
        "annotationInputToModel": False,
        "requestSha256": sha256_bytes(json.dumps(request_hashes, separators=(",", ":")).encode("utf-8")),
        "requestHashes": request_hashes,
        "sourceFile": str(input_path),
        "sourceSha256": sha256_file(input_path),
        "windowed": True,
        "windowSize": BOUNDARY_WINDOW_SEGMENTS,
        "windowOverlap": BOUNDARY_WINDOW_OVERLAP,
    }
    raw_entries: list[dict[str, Any]] = []
    window_metadata: list[dict[str, Any]] = []
    status = "completed"
    failure_reason: str | None = None
    for request, window, chunk in window_requests:
        try:
            window_status, predictions, metadata, reason, runner_output_sha256 = _run_external(
                request, runner, environment, timeout
            )
        except (DatasetEvalError, OSError, subprocess.TimeoutExpired) as error:
            status = "failed"
            failure_reason = str(error)
            break
        window_metadata.append({"window": window, "metadata": metadata, "runnerOutputSha256": runner_output_sha256})
        if window_status != "completed":
            status = window_status
            failure_reason = reason
            break
        try:
            validate_boundary_predictions([chunk], predictions)
        except DatasetEvalError as error:
            status = "failed"
            failure_reason = str(error)
            break
        raw_entries.append({
            "scenarioId": window["scenarioId"],
            "window": window,
            "transitions": predictions[0]["transitions"],
        })
    base.update({
        "status": status,
        "reason": failure_reason,
        "runnerMetadata": {
            "windowCount": len(window_requests),
            "completedWindowCount": len(raw_entries),
            "windows": window_metadata,
        },
    })
    assembled: list[dict[str, Any]] = []
    assembler_audit: list[dict[str, Any]] = []
    if base["status"] == "completed":
        try:
            merged: dict[str, dict[str, bool]] = {record["scenarioId"]: {} for record in records}
            conflict_count = 0
            duplicate_count = 0
            for entry in raw_entries:
                decision_map = merged[entry["scenarioId"]]
                for transition in entry["transitions"]:
                    after_id = transition["after_segment_id"]
                    boundary = transition["boundary"]
                    if after_id in decision_map:
                        duplicate_count += 1
                        if decision_map[after_id] != boundary:
                            conflict_count += 1
                    else:
                        decision_map[after_id] = boundary
            merged_predictions = []
            for record in records:
                segment_ids = [segment["id"] for segment in record["segments"]]
                decision_map = merged[record["scenarioId"]]
                expected = set(segment_ids[:-1])
                observed = set(decision_map)
                assembler_audit.append({
                    "scenarioId": record["scenarioId"],
                    "expectedTransitionCount": len(expected),
                    "observedTransitionCount": len(observed),
                    "missingTransitionIds": sorted(expected - observed),
                    "duplicateTransitionCount": duplicate_count,
                    "conflictCount": conflict_count,
                })
                if expected != observed or conflict_count:
                    raise DatasetEvalError("boundary window merge did not produce a conflict-free complete transition set")
                merged_predictions.append({
                    "scenarioId": record["scenarioId"],
                    "transitions": [
                        {"after_segment_id": segment_id, "boundary": decision_map[segment_id]}
                        for segment_id in segment_ids[:-1]
                    ],
                })
            assembled = assemble_boundary_predictions(records, merged_predictions)
            validate_dataset_predictions("ami", records, assembled)
            evaluation = evaluate_records(records, assembled)
            thresholds = quality_thresholds(False)
            passed = (
                evaluation["structuralValidityRate"] >= thresholds["structuralValidityRate"]
                and evaluation["evidenceValidityRate"] >= thresholds["evidenceValidityRate"]
                and evaluation["hierarchyValidityRate"] >= thresholds["hierarchyValidityRate"]
                and evaluation["boundaryF1"] >= thresholds["boundaryF1"]
                and evaluation["Pk"] <= thresholds["Pk"]
                and evaluation["WindowDiff"] <= thresholds["WindowDiff"]
            )
            base.update(
                {
                    "qualityStatus": "passed" if passed else "failed",
                    "evaluation": evaluation,
                    "thresholds": thresholds,
                    "rawBoundaryPredictionCount": len(raw_entries),
                    "assemblerAudit": assembler_audit,
                }
            )
        except (DatasetEvalError, KeyError, TypeError, ValueError) as error:
            base.update({"status": "failed", "reason": f"boundary-first evaluation failed: {error}"})
    assembled_bytes = b"".join(
        json.dumps(item, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8") + b"\n"
        for item in assembled
    )
    raw_bytes = b"".join(
        json.dumps(item, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8") + b"\n"
        for item in raw_entries
    )
    base["predictionSha256"] = sha256_bytes(assembled_bytes)
    base["rawBoundaryPredictionSha256"] = sha256_bytes(raw_bytes)
    base["assemblerAudit"] = assembler_audit
    if output_dir is not None:
        output_dir.mkdir(parents=True, exist_ok=True)
        (output_dir / "boundary-predictions.jsonl").write_bytes(raw_bytes)
        (output_dir / "prediction.jsonl").write_bytes(assembled_bytes)
        (output_dir / "report.json").write_text(
            json.dumps(base, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    return base


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dataset", choices=sorted(DATASET_FILES))
    parser.add_argument("--input", type=Path)
    parser.add_argument("--protocol", choices=sorted(PROTOCOLS), default="openaiResponses")
    parser.add_argument("--runner", default=DEFAULT_RUNNER)
    parser.add_argument("--limit", type=int, default=2)
    parser.add_argument("--single-topic-only", action="store_true")
    parser.add_argument("--boundary-first", action="store_true")
    parser.add_argument("--timeout", type=float, default=120.0)
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args()
    if args.limit <= 0:
        parser.error("--limit must be positive")
    dataset = args.dataset
    input_path = args.input
    if input_path is None:
        if dataset is None:
            parser.error("one of --dataset or --input is required")
        input_path = DATASET_FILES[dataset]
    elif dataset is None:
        dataset = input_path.stem
    if dataset not in DATASET_FILES:
        parser.error("--dataset must be qmsum or ami when supplied")
    output_dir = args.output_dir
    if output_dir is None:
        output_dir = DEFAULT_OUTPUT / dataset / args.protocol
    try:
        if args.boundary_first:
            if dataset != "ami":
                parser.error("--boundary-first currently supports only --dataset ami")
            report = run_boundary_first_dataset(
                input_path,
                protocol=args.protocol,
                runner=args.runner,
                limit=args.limit,
                environment=dict(os.environ),
                timeout=args.timeout,
                output_dir=output_dir,
            )
        else:
            report = run_dataset(
                dataset,
                input_path,
                protocol=args.protocol,
                runner=args.runner,
                limit=args.limit,
                single_topic_only=args.single_topic_only,
                environment=dict(os.environ),
                timeout=args.timeout,
                output_dir=output_dir,
            )
    except (DatasetEvalError, OSError, ValueError) as error:
        parser.error(str(error))
    print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if report["status"] in {"completed", "not_assessed"} else 2


if __name__ == "__main__":
    raise SystemExit(main())
