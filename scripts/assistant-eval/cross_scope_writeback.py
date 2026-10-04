#!/usr/bin/env python3
"""Submit one public cross-scope prediction through the real Workspace CLI.

This is a submission harness, not a semantic evaluator.  It never reads a
case's gold, negative cases, or rationale.  The only mutation evidence emitted
by this module comes from the Workspace CLI receipt and subsequent readbacks;
model metadata is never promoted to a mutation receipt.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shlex
import sys
from pathlib import Path
from typing import Any

from cross_scope_eval import (
    CrossScopeError,
    _normalize_prediction,
    _public_scope,
    _run_external,
    load_json,
    build_request,
)
from writeback import WritebackConfig, WritebackError, writeback_prediction


ROOT = Path(__file__).resolve().parents[2]
FIXTURE = Path(__file__).resolve().parent / "fixtures" / "cross-scope.json"
DEFAULT_REPORT = ROOT / "eval-data/assistant/data/external/cross-scope/openaiResponses/writeback-report.json"
DEFAULT_RUNNER = f"{sys.executable} {Path(__file__).resolve().parent / 'runner.py'}"
DEFAULT_CLI = ROOT / "target/debug/meetnote"


def _segments(fixture: dict[str, Any], case: dict[str, Any]) -> tuple[list[str], dict[str, dict[str, Any]]]:
    scope = _public_scope(fixture, case)
    order: list[str] = []
    by_id: dict[str, dict[str, Any]] = {}
    for document in scope["documents"]:
        for block in document["blocks"]:
            for segment in block["segments"]:
                segment_id = segment["segmentId"]
                order.append(segment_id)
                by_id[segment_id] = {
                    "documentId": document["documentId"],
                    "blockId": block["blockId"],
                    "text": segment["text"],
                }
    return order, by_id


def _public_document_record(fixture: dict[str, Any], case: dict[str, Any]) -> dict[str, Any]:
    order, by_id = _segments(fixture, case)
    transcript = [
        {
            "id": segment_id,
            "start_ms": index * 1000,
            "end_ms": (index + 1) * 1000,
            "text": by_id[segment_id]["text"],
        }
        for index, segment_id in enumerate(order)
    ]
    markdown = "# Cross-scope public submission\n\n" + "\n".join(
        f"- [{item['id']}] {item['text']}" for item in transcript
    ) + "\n"
    state = {
        "schema": {"schema_version": 2, "revision": 0, "updated_at_ms": 0},
        "identity": {"id": "cross-scope-public-parent", "created_at_ms": 0},
        "source": {
            "audio_path": "fixture://cross-scope-public",
            "audio_start_ms": 0,
            "audio_end_ms": len(transcript) * 1000,
            "audio_asset_id": "cross-scope-public",
            "recorded_at_ms": None,
        },
        "hierarchy": {"parent_id": None, "depth": 0, "sibling_order": 0},
        "document_status": {"value": "ready"},
        "content": {"title": "Cross-scope public submission", "tags": [], "summary": None},
        "processing": {"latest": None, "last_error": None},
        "results": {
            "transcript": transcript,
            "diarization": [],
            "speaker_segments": [],
            "markdown_path": None,
        },
    }
    return {"scenarioId": case["caseId"], "record": {"markdown": markdown, "state": state}}


def _writeback_children(
    prediction: dict[str, Any], order: list[str], by_id: dict[str, dict[str, Any]]
) -> list[dict[str, Any]]:
    if prediction.get("mode") != "partition":
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "cross-scope submission requires partition mode")
    if prediction.get("uncoveredSpans") or prediction.get("uncovered_ranges"):
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "partition submission cannot contain uncovered ranges")
    children = prediction.get("children")
    if not isinstance(children, list) or not children:
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "partition prediction children are empty")
    positions = {segment_id: index for index, segment_id in enumerate(order)}
    covered: set[str] = set()
    result: list[dict[str, Any]] = []
    for index, child in enumerate(children):
        if not isinstance(child, dict):
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"child {index} is not an object")
        spans = child.get("spans")
        if not isinstance(spans, list) or not spans:
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"child {index} has no spans")
        segment_ids = [segment_id for span in spans for segment_id in span.get("segmentIds", [])]
        if not segment_ids or any(segment_id not in positions for segment_id in segment_ids):
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"child {index} has unknown segment IDs")
        child_positions = sorted(positions[segment_id] for segment_id in segment_ids)
        if child_positions != list(range(child_positions[0], child_positions[-1] + 1)):
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"child {index} spans are not contiguous")
        if covered.intersection(segment_ids):
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"child {index} overlaps another child")
        covered.update(segment_ids)
        evidence = child.get("evidenceSegmentIds", child.get("evidence_segment_ids"))
        if not isinstance(evidence, list) or not evidence or not set(evidence) <= set(segment_ids):
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"child {index} evidence is invalid")
        title = child.get("title")
        summary = child.get("summary")
        tags = child.get("tags", [])
        if not isinstance(title, str) or not title.strip() or not isinstance(summary, str) or not summary.strip():
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"child {index} requires model title and summary for writeback")
        if not isinstance(tags, list) or not all(isinstance(tag, str) and tag.strip() for tag in tags):
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"child {index} tags are invalid")
        result.append(
            {
                "start_segment_id": order[child_positions[0]],
                "end_segment_id": order[child_positions[-1]],
                "evidence_segment_ids": list(evidence),
                "title": title,
                "summary": summary,
                "tags": list(tags),
            }
        )
    if covered != set(order):
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "partition submission does not cover the public transcript")
    return result


def _submission_request(fixture: dict[str, Any], case: dict[str, Any], protocol: str) -> dict[str, Any]:
    request = build_request(fixture, case, protocol)
    request["prompt"]["task"] += (
        " For this real writeback submission, every child must also include a "
        "non-empty title, summary, and tags array; these fields are required by "
        "the Workspace create_semantic_children contract."
    )
    return request


def run_submission(
    fixture: dict[str, Any],
    *,
    runner: str,
    protocol: str,
    cli: str,
    package_root: Path,
) -> dict[str, Any]:
    case = next((item for item in fixture.get("cases", []) if item.get("caseId") == "cross-block-partition"), None)
    if not isinstance(case, dict):
        raise CrossScopeError("FIXTURE_INVALID", "cross-block-partition case is missing")
    request = _submission_request(fixture, case, protocol)
    try:
        result = _run_external(shlex.split(runner), request, timeout=120.0)
        matching = [item for item in result["predictions"] if isinstance(item, dict) and item.get("scenarioId") == case["caseId"]]
        if len(matching) != 1:
            raise CrossScopeError("PREDICTION_COVERAGE_INVALID", "submission runner did not return exactly one partition prediction")
        prediction = _normalize_prediction(fixture, case, matching[0])
        order, by_id = _segments(fixture, case)
        prediction_for_writeback = {
            "scenarioId": prediction["scenarioId"],
            "mode": prediction.get("mode"),
            "children": _writeback_children(prediction, order, by_id),
            "uncovered_ranges": [],
        }
        prediction_hash = hashlib.sha256(json.dumps(prediction, ensure_ascii=False, sort_keys=True).encode("utf-8")).hexdigest()
        evidence = writeback_prediction(
            prediction_for_writeback,
            _public_document_record(fixture, case),
            WritebackConfig(cli=shlex.split(cli), package_root=package_root),
            family="partition",
            run_key="cross-scope-partition",
        )
        receipts = evidence["mutationReceipts"]
        if len(receipts) != 1 or not receipts[0].get("mutationId"):
            raise WritebackError("WORKSPACE_RECEIPT_INVALID", "submission receipt lacks mutationId")
        parent_id = evidence["documentId"]
        parent_revision = next((item for item in receipts[0]["affectedDocuments"] if item.get("documentId") == parent_id), None)
        if not isinstance(parent_revision, dict) or parent_revision.get("previousRevision") != 0 or parent_revision.get("revision") != 1:
            raise WritebackError("WORKSPACE_RECEIPT_INVALID", "parent affected revision did not advance from 0 to 1")
        return {
            "schemaVersion": 1,
            "status": "completed",
            "protocol": protocol,
            "caseId": case["caseId"],
            "predictionSource": "external_runner",
            "predictionSha256": prediction_hash,
            "mutationReceipts": receipts,
            "readback": evidence["readback"],
            "readbackSha256": evidence["readbackSha256"],
        }
    except (CrossScopeError, WritebackError) as error:
        report: dict[str, Any] = {
            "schemaVersion": 1,
            "status": "not_assessed" if isinstance(error, CrossScopeError) and error.status == "not_assessed" else "failed",
            "protocol": protocol,
            "caseId": case["caseId"],
            "predictionSource": "external_runner",
            "errorCode": error.code,
            "reason": str(error),
        }
        if isinstance(error, WritebackError):
            report["mutationReceipts"] = error.receipts
            report["readback"] = error.readback
        return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runner", default=DEFAULT_RUNNER)
    parser.add_argument("--protocol", default="openaiResponses")
    parser.add_argument("--cli", default=str(DEFAULT_CLI))
    parser.add_argument("--fixture", type=Path, default=FIXTURE)
    parser.add_argument("--package-root", type=Path, default=ROOT / "eval-data/assistant/writeback")
    parser.add_argument("--report", type=Path, default=DEFAULT_REPORT)
    args = parser.parse_args(argv)
    report = run_submission(
        load_json(args.fixture),
        runner=args.runner,
        protocol=args.protocol,
        cli=args.cli,
        package_root=args.package_root,
    )
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
