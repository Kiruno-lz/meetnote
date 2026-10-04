#!/usr/bin/env python3
"""Strict, model-independent schema checks for semantic evaluation fixtures.

The fixture in this module is intentionally small.  It checks identity and
coverage across document/block boundaries without pretending that its gold
labels are an independent model prediction.
"""

from __future__ import annotations

from hashlib import sha256
from typing import Any


ALLOWED_MODES = {"partition", "extract", "none"}


class SemanticSchemaError(ValueError):
    """Raised when a semantic fixture, prediction, or evidence chain is invalid."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise SemanticSchemaError(message)


def _unique_strings(values: Any, label: str) -> list[str]:
    _require(isinstance(values, list) and values, f"{label} must be a non-empty array")
    _require(all(isinstance(value, str) and value for value in values), f"{label} must contain non-empty strings")
    _require(len(values) == len(set(values)), f"{label} contains duplicates")
    return values


def _scope_maps(fixture: dict[str, Any]) -> tuple[dict[str, dict[str, Any]], dict[str, dict[str, Any]], dict[str, str]]:
    documents: dict[str, dict[str, Any]] = {}
    blocks: dict[str, dict[str, Any]] = {}
    segments: dict[str, str] = {}
    for document in fixture["documents"]:
        document_id = document["documentId"]
        documents[document_id] = document
        for block in document["blocks"]:
            block_id = block["blockId"]
            blocks[block_id] = block
            for segment in block["segments"]:
                segments[segment["segmentId"]] = block_id
    return documents, blocks, segments


def _case_segment_ids(case: dict[str, Any], blocks: dict[str, dict[str, Any]]) -> list[str]:
    result: list[str] = []
    for block_id in case["inputBlockIds"]:
        result.extend(segment["segmentId"] for segment in blocks[block_id]["segments"])
    return result


def _validate_spans(
    spans: Any,
    *,
    label: str,
    case_segment_ids: list[str],
    blocks: dict[str, dict[str, Any]],
    segments: dict[str, str],
) -> tuple[list[str], list[str]]:
    _require(isinstance(spans, list), f"{label} must be an array")
    case_set = set(case_segment_ids)
    covered: list[str] = []
    first_positions: list[int] = []
    position = {segment_id: index for index, segment_id in enumerate(case_segment_ids)}
    for index, span in enumerate(spans):
        _require(isinstance(span, dict), f"{label}[{index}] must be an object")
        document_id = span.get("documentId")
        block_id = span.get("blockId")
        segment_ids = _unique_strings(span.get("segmentIds"), f"{label}[{index}].segmentIds")
        _require(block_id in blocks, f"{label}[{index}] references unknown block")
        _require(all(segment_id in segments for segment_id in segment_ids), f"{label}[{index}] references unknown segment")
        _require(all(segment_id in case_set for segment_id in segment_ids), f"{label}[{index}] references a segment outside the case")
        _require(all(segments[segment_id] == block_id for segment_id in segment_ids), f"{label}[{index}] crosses block ownership")
        _require(all(blocks[block_id]["documentId"] == document_id for _ in [0]), f"{label}[{index}] has wrong document ownership")
        _require(not set(covered) & set(segment_ids), f"{label} contains duplicate or overlapping segments")
        covered.extend(segment_ids)
        first_positions.append(min(position[segment_id] for segment_id in segment_ids))
    _require(first_positions == sorted(first_positions), f"{label} is out of input order")
    return covered, first_positions


def _validate_prediction_shape(
    case: dict[str, Any],
    prediction: dict[str, Any],
    *,
    blocks: dict[str, dict[str, Any]],
    segments: dict[str, str],
    label: str,
) -> None:
    _require(isinstance(prediction, dict), f"{label} must be an object")
    _require(prediction.get("caseId") == case["caseId"], f"{label}.caseId does not match case")
    mode = prediction.get("mode")
    _require(mode in ALLOWED_MODES, f"{label}.mode is invalid")
    case_segment_ids = _case_segment_ids(case, blocks)
    children = prediction.get("children")
    uncovered = prediction.get("uncoveredSpans", [])
    _require(isinstance(children, list), f"{label}.children must be an array")
    child_ids = [child.get("childId") for child in children if isinstance(child, dict)]
    _require(len(child_ids) == len(children) and all(isinstance(value, str) and value for value in child_ids), f"{label}.children must have childId")
    _require(len(child_ids) == len(set(child_ids)), f"{label}.children contains duplicate childId")
    covered, _ = _validate_spans(
        [span for child in children for span in child.get("spans", [])],
        label=f"{label}.children.spans",
        case_segment_ids=case_segment_ids,
        blocks=blocks,
        segments=segments,
    )
    for child in children:
        child_spans = child.get("spans")
        _require(isinstance(child_spans, list) and child_spans, f"{label}.children[{child['childId']}].spans must be non-empty")
        evidence = _unique_strings(child.get("evidenceSegmentIds"), f"{label}.children[{child['childId']}].evidenceSegmentIds")
        child_segments = [segment_id for span in child_spans for segment_id in span["segmentIds"]]
        _require(set(evidence) <= set(child_segments), f"{label}.children[{child['childId']}] evidence escapes its spans")

    uncovered_ids, _ = _validate_spans(
        uncovered,
        label=f"{label}.uncoveredSpans",
        case_segment_ids=case_segment_ids,
        blocks=blocks,
        segments=segments,
    )
    _require(not set(covered) & set(uncovered_ids), f"{label} children overlap uncovered spans")
    all_ids = set(case_segment_ids)
    if mode == "partition":
        _require(set(covered) == all_ids and not uncovered_ids, f"{label} partition omits or duplicates input segments")
    elif mode == "extract":
        _require(set(covered) | set(uncovered_ids) == all_ids, f"{label} extract has an omission")
    else:
        _require(not children and not uncovered, f"{label} none mode must contain no spans")
        rationale = prediction.get("semanticRationale")
        _require(isinstance(rationale, dict), f"{label}.semanticRationale is required for none mode")
        _require(rationale.get("topicSupport") == "insufficient", f"{label}.none rationale must report insufficient topic support")
        rationale_ids = _unique_strings(rationale.get("evidenceSegmentIds"), f"{label}.semanticRationale.evidenceSegmentIds")
        _require(set(rationale_ids) <= set(case_segment_ids), f"{label}.none rationale escapes the input scope")

    second_pass = prediction.get("secondPass")
    if second_pass is not None:
        _require(mode == "partition", f"{label}.secondPass requires a partition")
        _require(isinstance(second_pass, dict), f"{label}.secondPass must be an object")
        parent_id = second_pass.get("parentChildId")
        _require(parent_id in child_ids, f"{label}.secondPass parentChildId is unknown")
        parent = next(child for child in children if child["childId"] == parent_id)
        parent_ids = {segment_id for span in parent["spans"] for segment_id in span["segmentIds"]}
        parent_block_ids = []
        for span in parent["spans"]:
            if span["blockId"] not in parent_block_ids:
                parent_block_ids.append(span["blockId"])
        nested = dict(second_pass)
        nested["caseId"] = case["caseId"]
        _validate_prediction_shape(
            {
                **case,
                "inputBlockIds": parent_block_ids,
                "inputDocumentIds": list(dict.fromkeys(blocks[block_id]["documentId"] for block_id in parent_block_ids)),
            },
            nested,
            blocks=blocks,
            segments=segments,
            label=f"{label}.secondPass",
        )
        nested_ids = {
            segment_id
            for child in second_pass.get("children", [])
            for span in child.get("spans", [])
            for segment_id in span.get("segmentIds", [])
        }
        nested_gap_ids = {
            segment_id
            for span in second_pass.get("uncoveredSpans", [])
            for segment_id in span.get("segmentIds", [])
        }
        _require(nested_ids | nested_gap_ids == parent_ids, f"{label}.secondPass does not preserve parent coverage")
        _require(nested_ids <= parent_ids and nested_gap_ids <= parent_ids, f"{label}.secondPass escapes parent range")


def validate_semantic_fixture(fixture: dict[str, Any]) -> dict[str, Any]:
    _require(fixture.get("schemaVersion") == 1, "semantic fixture schemaVersion must be 1")
    _require(fixture.get("fixtureType") == "assistant-semantic-cross-scope", "semantic fixture type is invalid")
    _require(fixture.get("independentModelEvidence") is False, "fixture must declare independentModelEvidence=false")
    documents = fixture.get("documents")
    cases = fixture.get("cases")
    _require(isinstance(documents, list) and documents, "semantic fixture documents are missing")
    _require(isinstance(cases, list) and cases, "semantic fixture cases are missing")
    document_ids = [document.get("documentId") for document in documents]
    _require(all(isinstance(value, str) and value for value in document_ids), "document IDs are invalid")
    _require(len(document_ids) == len(set(document_ids)), "document IDs contain duplicates")
    fixture["documents"] = documents
    blocks: dict[str, dict[str, Any]] = {}
    segments: dict[str, str] = {}
    for document in documents:
        document_id = document["documentId"]
        document_blocks = document.get("blocks")
        _require(isinstance(document_blocks, list) and document_blocks, f"{document_id} blocks are missing")
        for block in document_blocks:
            block_id = block.get("blockId")
            _require(isinstance(block_id, str) and block_id and block_id not in blocks, f"invalid or duplicate blockId: {block_id}")
            block["documentId"] = document_id
            blocks[block_id] = block
            block_segments = block.get("segments")
            _require(isinstance(block_segments, list) and block_segments, f"{block_id} segments are missing")
            for segment in block_segments:
                segment_id = segment.get("segmentId")
                _require(isinstance(segment_id, str) and segment_id and segment_id not in segments, f"invalid or duplicate segmentId: {segment_id}")
                segments[segment_id] = block_id

    case_ids = [case.get("caseId") for case in cases]
    _require(all(isinstance(value, str) and value for value in case_ids), "case IDs are invalid")
    _require(len(case_ids) == len(set(case_ids)), "case IDs contain duplicates")
    modes: set[str] = set()
    has_cross_block = False
    has_cross_document = False
    has_second_pass = False
    for case in cases:
        _require(isinstance(case.get("inputDocumentIds"), list), f"{case['caseId']} inputDocumentIds is missing")
        _require(isinstance(case.get("inputBlockIds"), list) and case["inputBlockIds"], f"{case['caseId']} inputBlockIds is missing")
        _require(set(case["inputDocumentIds"]) <= set(document_ids), f"{case['caseId']} references an unknown document")
        _require(set(case["inputBlockIds"]) <= set(blocks), f"{case['caseId']} references an unknown block")
        _require(all(blocks[block_id]["documentId"] in case["inputDocumentIds"] for block_id in case["inputBlockIds"]), f"{case['caseId']} block/document scope mismatch")
        has_cross_block |= len(case["inputBlockIds"]) > 1
        has_cross_document |= len(case["inputDocumentIds"]) > 1
        gold = case.get("gold")
        _require(isinstance(gold, dict), f"{case['caseId']} gold is missing")
        gold_prediction = {"caseId": case["caseId"], **gold}
        if gold["mode"] == "none":
            gold_prediction["semanticRationale"] = case["semanticRationale"]
        _validate_prediction_shape(case, gold_prediction, blocks=blocks, segments=segments, label=f"gold[{case['caseId']}]")
        modes.add(gold["mode"])
        has_second_pass |= gold.get("secondPass") is not None
        if gold["mode"] == "none":
            rationale = case.get("semanticRationale")
            _require(isinstance(rationale, dict), f"{case['caseId']} none case lacks semantic rationale")
            _require(rationale.get("topicSupport") == "insufficient", f"{case['caseId']} none rationale is not an insufficient-evidence decision")
            _require(rationale.get("instructionIndependent") is True, f"{case['caseId']} none rationale relies only on the user directive")
            rationale_ids = _unique_strings(rationale.get("evidenceSegmentIds"), f"{case['caseId']}.semanticRationale.evidenceSegmentIds")
            _require(set(rationale_ids) <= set(_case_segment_ids(case, blocks)), f"{case['caseId']} none rationale escapes the input scope")
        negative_cases = case.get("negativeCases")
        _require(isinstance(negative_cases, list), f"{case['caseId']} negativeCases are missing")
        expected_errors = {item.get("errorCode") for item in negative_cases if isinstance(item, dict)}
        _require({"DUPLICATE_SEGMENT", "OMITTED_SEGMENT"} <= expected_errors, f"{case['caseId']} lacks duplicate/omission negatives")
    _require(has_cross_block, "fixture must include a cross-block case")
    _require(has_cross_document, "fixture must include a cross-document case")
    _require(has_second_pass, "fixture must include a second-pass case")
    _require(modes == ALLOWED_MODES, "fixture must cover partition, extract, and none")
    return {"documentCount": len(documents), "blockCount": len(blocks), "segmentCount": len(segments), "caseCount": len(cases), "modes": sorted(modes)}


def validate_semantic_prediction(fixture: dict[str, Any], prediction: dict[str, Any]) -> None:
    """Validate one external prediction against a fixture case without scoring it."""
    validate_semantic_fixture(fixture)
    cases = {case["caseId"]: case for case in fixture["cases"]}
    case_id = prediction.get("caseId")
    _require(case_id in cases, "prediction references an unknown case")
    _, blocks, segments = _scope_maps(fixture)
    _validate_prediction_shape(cases[case_id], prediction, blocks=blocks, segments=segments, label=f"prediction[{case_id}]")


def validate_evidence_chain(evidence: dict[str, Any], *, expected_case_id: str, prediction_bytes: bytes | None = None) -> None:
    """Validate metadata, receipt, readback, and correction provenance."""
    _require(evidence.get("schemaVersion") == 1, "evidence schemaVersion must be 1")
    metadata = evidence.get("metadata")
    receipt = evidence.get("receipt")
    readback = evidence.get("readback")
    prediction = evidence.get("prediction")
    for name, value in (("metadata", metadata), ("receipt", receipt), ("readback", readback), ("prediction", prediction)):
        _require(isinstance(value, dict), f"evidence {name} is missing")
    run_id = metadata.get("runId")
    _require(isinstance(run_id, str) and run_id, "metadata.runId is missing")
    _require(metadata.get("caseId") == expected_case_id, "metadata.caseId does not match case")
    _require(receipt.get("runId") == run_id and readback.get("runId") == run_id, "receipt/readback runId mismatch")
    _require(metadata.get("predictionSource") == "external_runner", "predictionSource must identify an external runner")
    _require("oracle" not in str(metadata.get("predictionSource")).lower(), "oracle evidence is not independent")
    _require(isinstance(metadata.get("model"), str) and metadata["model"], "metadata.model is missing")
    _require(isinstance(metadata.get("promptSha256"), str) and len(metadata["promptSha256"]) == 64, "metadata.promptSha256 is missing")
    _require(isinstance(metadata.get("resources"), dict), "metadata.resources is missing")
    _require(isinstance(metadata.get("elapsedMs"), int) and metadata["elapsedMs"] >= 0, "metadata.elapsedMs is invalid")
    _require(isinstance(receipt.get("mutationReceipts"), list), "receipt.mutationReceipts is missing")
    _require(isinstance(readback.get("mutationReceipts"), list), "readback.mutationReceipts is missing")
    _require(receipt["mutationReceipts"] == readback["mutationReceipts"], "receipt/readback mutation receipts differ")
    _require(readback.get("predictionSha256") == receipt.get("predictionSha256"), "prediction hash differs between receipt/readback")
    if prediction_bytes is not None:
        digest = sha256(prediction_bytes).hexdigest()
        _require(readback.get("predictionSha256") == digest, "predictionSha256 does not match prediction bytes")
    if expected_case_id == "validator_correction":
        correction = prediction.get("correction")
        _require(isinstance(correction, dict), "validator prediction correction is missing")
        _require(isinstance(correction.get("errorCode"), str) and correction["errorCode"], "validator correction errorCode is missing")
        _require(isinstance(correction.get("correctedOperation"), dict), "validator correction correctedOperation is missing")
        _require(correction.get("retry") is False, "validator correction must not retry blindly")
