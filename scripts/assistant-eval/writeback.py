#!/usr/bin/env python3
"""Exercise the real Workspace capability CLI with public eval evidence only.

This module is intentionally independent from the model runner.  It receives a
validated external prediction and the public ``record`` payload, creates a
throw-away document package, invokes the Workspace CLI, and verifies the
mutation receipt against subsequent document reads.  It never looks at corpus
gold labels, negative cases, or semantic rationale.
"""

from __future__ import annotations

import copy
import hashlib
import json
import os
import shlex
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence


PROJECT_ROOT = Path(__file__).resolve().parents[2]
EVAL_DATA_ROOT = (PROJECT_ROOT / "eval-data").resolve()
SYSTEM_TEMP_ROOT = Path(tempfile.gettempdir()).resolve()

# Evaluator annotations remain forbidden.  Model-generated ``rationale`` and
# ``semanticRationale`` are intentionally allowed and ignored by writeback.
FORBIDDEN_PREDICTION_KEYS = frozenset(
    {
        "gold",
        "negativecase",
        "negativecases",
        "annotation",
        "annotations",
        "topichierarchy",
        "queries",
        "answer",
        "relevant_text_span",
        "relevanttextspan",
    }
)
WRITEBACK_FAMILIES = frozenset({"partition", "organize"})


class WritebackError(RuntimeError):
    """A structured failure while invoking or validating Workspace writeback."""

    def __init__(
        self,
        code: str,
        reason: str,
        *,
        receipts: list[dict[str, Any]] | None = None,
        readback: list[dict[str, Any]] | None = None,
    ) -> None:
        super().__init__(reason)
        self.code = code
        self.reason = reason
        self.receipts = receipts or []
        self.readback = readback or []


@dataclass(frozen=True)
class WritebackConfig:
    """Explicit inputs for a real Workspace CLI run."""

    cli: str | Sequence[str]
    package_root: Path
    keep_packages: bool = False
    timeout: float = 120.0
    environment: Mapping[str, str] | None = None


def _sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def markdown_sha256(markdown: str) -> str:
    return _sha256_bytes(markdown.encode("utf-8"))


def _json_size(value: Any) -> int:
    return len(json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))


def _cli_args(cli: str | Sequence[str]) -> list[str]:
    if isinstance(cli, str):
        command = shlex.split(cli)
    else:
        command = [str(item) for item in cli]
    if not command or any(not item for item in command):
        raise WritebackError("WORKSPACE_CLI_INVALID", "workspace CLI command is empty")
    return command


def _public_record(record: Mapping[str, Any]) -> Mapping[str, Any]:
    """Return only the public synthetic record without inspecting evaluator data."""

    value = record.get("record")
    if not isinstance(value, Mapping):
        raise WritebackError("PUBLIC_RECORD_INVALID", "synthetic record.public payload is missing")
    return value


def _reject_evaluator_prediction_fields(value: Any, path: str = "$") -> None:
    """Reject copied evaluator labels while allowing model rationale fields."""

    if isinstance(value, Mapping):
        for key, child in value.items():
            if not isinstance(key, str):
                raise WritebackError(
                    "EVALUATOR_DATA_IN_PREDICTION",
                    f"{path} contains a non-string prediction field",
                )
            if key.lower() in FORBIDDEN_PREDICTION_KEYS:
                raise WritebackError(
                    "EVALUATOR_DATA_IN_PREDICTION",
                    f"{path}.{key} is evaluator-only data",
                )
            _reject_evaluator_prediction_fields(child, f"{path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            _reject_evaluator_prediction_fields(child, f"{path}[{index}]")


def _document_id(public_record: Mapping[str, Any], run_key: str) -> str:
    state = public_record.get("state")
    identity = state.get("identity") if isinstance(state, Mapping) else None
    original = identity.get("id") if isinstance(identity, Mapping) else None
    if not isinstance(original, str) or not original:
        raise WritebackError("PUBLIC_RECORD_INVALID", "public record state.identity.id is missing")
    digest = hashlib.sha256(f"{run_key}:{original}".encode("utf-8")).hexdigest()[:16]
    return f"assistant-eval-{digest}"


def create_temporary_document_package(
    record: Mapping[str, Any], package_root: Path, run_key: str
) -> tuple[tempfile.TemporaryDirectory[str], Path, str]:
    """Create one isolated package from ``record.record`` and return its context."""

    resolved_package_root = package_root.expanduser().resolve()
    if not any(
        resolved_package_root == allowed
        or allowed in resolved_package_root.parents
        for allowed in (EVAL_DATA_ROOT, SYSTEM_TEMP_ROOT)
    ):
        raise WritebackError(
            "WRITEBACK_PATH_INVALID",
            "writeback package root must be under the repository eval-data/ or system temporary directory",
        )

    public_record = _public_record(record)
    state = public_record.get("state")
    markdown = public_record.get("markdown")
    if not isinstance(state, Mapping) or not isinstance(markdown, str):
        raise WritebackError("PUBLIC_RECORD_INVALID", "public record must contain state and markdown")
    state_copy = copy.deepcopy(dict(state))
    document_id = _document_id(public_record, run_key)
    identity = state_copy.setdefault("identity", {})
    schema = state_copy.setdefault("schema", {})
    hierarchy = state_copy.setdefault("hierarchy", {})
    if not isinstance(identity, dict) or not isinstance(schema, dict) or not isinstance(hierarchy, dict):
        raise WritebackError("PUBLIC_RECORD_INVALID", "public record state has invalid metadata")
    identity["id"] = document_id
    schema["revision"] = 0
    schema["updated_at_ms"] = 0
    hierarchy["parent_id"] = None
    hierarchy["depth"] = 0
    hierarchy["sibling_order"] = 0

    resolved_package_root.mkdir(parents=True, exist_ok=True)
    context = tempfile.TemporaryDirectory(prefix=f"writeback-{run_key}-", dir=resolved_package_root)
    workspace_root = Path(context.name)
    document_dir = workspace_root / "data" / "documents" / document_id
    document_dir.mkdir(parents=True)
    (document_dir / "state.json").write_text(
        json.dumps(state_copy, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    (document_dir / "document.md").write_text(markdown, encoding="utf-8")
    return context, workspace_root, document_id


def _response_json(completed: subprocess.CompletedProcess[str], operation: str) -> dict[str, Any]:
    try:
        response = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise WritebackError(
            "WORKSPACE_CLI_INVALID_JSON",
            f"{operation} returned invalid JSON: {error}",
        ) from error
    if not isinstance(response, dict):
        raise WritebackError("WORKSPACE_CLI_INVALID_JSON", f"{operation} response is not an object")
    if not response.get("ok"):
        error = response.get("error")
        if isinstance(error, Mapping):
            code = str(error.get("code") or "WORKSPACE_TOOL_ERROR")
            message = str(error.get("message") or "workspace tool failed")
        else:
            code = "WORKSPACE_TOOL_ERROR"
            message = "workspace tool failed without a structured error"
        raise WritebackError(code, f"{operation}: {message}")
    return response


def _invoke(
    config: WritebackConfig,
    workspace_root: Path,
    document_id: str,
    operation: str,
    request: dict[str, Any],
) -> dict[str, Any]:
    command = _cli_args(config.cli)
    if operation == "read_document":
        tool_args = ["agent", "call", operation, "--json-stdin", "--document", document_id]
    else:
        tool_args = ["agent", "call", operation, "--json-stdin", "--document", document_id]
    environment = dict(config.environment or os.environ)
    try:
        completed = subprocess.run(
            command + tool_args,
            input=json.dumps(request, ensure_ascii=False),
            capture_output=True,
            text=True,
            cwd=workspace_root,
            env=environment,
            timeout=config.timeout,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise WritebackError("WORKSPACE_CLI_TIMEOUT", f"{operation} exceeded writeback timeout") from error
    except OSError as error:
        raise WritebackError("WORKSPACE_CLI_EXECUTION_FAILED", f"{operation} could not run: {error}") from error
    if completed.returncode not in (0, 2):
        detail = completed.stderr.strip() or f"exit code {completed.returncode}"
        raise WritebackError("WORKSPACE_CLI_EXECUTION_FAILED", f"{operation}: {detail}")
    return _response_json(completed, operation)


def _read_document(
    config: WritebackConfig, workspace_root: Path, document_id: str, request_id: str
) -> dict[str, Any]:
    request = {
        "schemaVersion": 1,
        "requestId": request_id,
        "toolCallId": f"call-{request_id}",
        "scope": {"documentIds": [document_id]},
        "arguments": {"documentId": document_id, "projection": "complete"},
    }
    response = _invoke(config, workspace_root, document_id, "read_document", request)
    result = response.get("result")
    if not isinstance(result, Mapping) or result.get("type") != "document":
        raise WritebackError("WORKSPACE_READBACK_INVALID", "read_document did not return a document")
    payload = result.get("payload")
    if not isinstance(payload, Mapping):
        raise WritebackError("WORKSPACE_READBACK_INVALID", "read_document payload is missing")
    metadata = payload.get("metadata")
    markdown = payload.get("markdown")
    if not isinstance(metadata, Mapping) or not isinstance(markdown, str):
        raise WritebackError("WORKSPACE_READBACK_INVALID", "complete readback lacks metadata or markdown")
    expected_hash = metadata.get("markdownSha256")
    actual_hash = markdown_sha256(markdown)
    normalized_hash = markdown_sha256(markdown[:-1]) if markdown.endswith("\n") else None
    if expected_hash not in {actual_hash, normalized_hash}:
        raise WritebackError(
            "WORKSPACE_READBACK_HASH_MISMATCH",
            f"{document_id} markdown hash does not match complete readback",
        )
    return {"documentId": document_id, "metadata": dict(metadata), "markdown": markdown}


def _scalar(value: Any, field: str) -> str:
    if isinstance(value, str) and value.strip():
        return value
    if isinstance(value, list) and value and isinstance(value[0], str) and value[0].strip():
        return value[0]
    raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"prediction child {field} must be non-empty")


def _tags(value: Any) -> list[str]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "prediction child tags must be an array")
    if all(isinstance(item, str) and item.strip() for item in value):
        return list(value)
    if value and isinstance(value[0], list) and all(isinstance(item, str) and item.strip() for item in value[0]):
        return list(value[0])
    raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "prediction child tags must contain strings")


def _semantic_children(prediction: Mapping[str, Any]) -> list[dict[str, Any]]:
    children = prediction.get("children")
    if not isinstance(children, list) or not children:
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "partition prediction children are empty")
    result = []
    for index, child in enumerate(children):
        if not isinstance(child, Mapping):
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"prediction child {index} is not an object")
        start = child.get("start_segment_id")
        end = child.get("end_segment_id")
        evidence = child.get("evidence_segment_ids", [])
        if not isinstance(start, str) or not start or not isinstance(end, str) or not end:
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"prediction child {index} lacks segment bounds")
        if not isinstance(evidence, list) or not all(isinstance(item, str) and item for item in evidence):
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"prediction child {index} evidence is invalid")
        result.append(
            {
                "startSegmentId": start,
                "endSegmentId": end,
                "evidenceSegmentIds": evidence,
                "title": _scalar(child.get("title"), "title"),
                "summary": _scalar(child.get("summary"), "summary"),
                "tags": _tags(child.get("tags", [])),
            }
        )
    return result


def _receipt(response: Mapping[str, Any], operation: str) -> dict[str, Any]:
    result = response.get("result")
    if not isinstance(result, Mapping) or result.get("type") != "mutation":
        if isinstance(result, Mapping) and result.get("type") == "pending":
            raise WritebackError("WORKSPACE_CONFIRMATION_REQUIRED", f"{operation} requires confirmation")
        raise WritebackError("WORKSPACE_RECEIPT_INVALID", f"{operation} did not return a mutation receipt")
    payload = result.get("payload")
    if not isinstance(payload, Mapping):
        raise WritebackError("WORKSPACE_RECEIPT_INVALID", f"{operation} mutation payload is missing")
    mutation_id = payload.get("mutationId")
    affected = payload.get("affectedDocuments")
    if not isinstance(mutation_id, str) or not mutation_id:
        raise WritebackError("WORKSPACE_RECEIPT_INVALID", f"{operation} receipt lacks mutationId")
    if not isinstance(affected, list) or not affected:
        raise WritebackError("WORKSPACE_RECEIPT_INVALID", f"{operation} receipt lacks affectedDocuments")
    return dict(payload)


def _expected_document_ids(receipt: Mapping[str, Any]) -> list[str]:
    affected = receipt.get("affectedDocuments")
    return [item["documentId"] for item in affected if isinstance(item, Mapping) and isinstance(item.get("documentId"), str)]


def _verify_receipt_readback(
    receipt: Mapping[str, Any],
    readbacks: list[dict[str, Any]],
    *,
    expected_revision: int,
    expected_created_count: int | None = None,
) -> None:
    affected = receipt.get("affectedDocuments")
    if not isinstance(affected, list):
        raise WritebackError("WORKSPACE_RECEIPT_INVALID", "affectedDocuments is not an array")
    if expected_created_count is not None and len(affected) != expected_created_count:
        raise WritebackError(
            "WORKSPACE_RECEIPT_INVALID",
            f"receipt affectedDocuments count {len(affected)} != {expected_created_count}",
        )
    by_id = {item["documentId"]: item for item in readbacks}
    if set(by_id) != set(_expected_document_ids(receipt)):
        raise WritebackError("WORKSPACE_READBACK_INVALID", "readback documents do not match receipt")
    for item in affected:
        if not isinstance(item, Mapping):
            raise WritebackError("WORKSPACE_RECEIPT_INVALID", "receipt contains an invalid document revision")
        document_id = item.get("documentId")
        readback = by_id.get(document_id)
        if readback is None:
            raise WritebackError("WORKSPACE_READBACK_INVALID", f"missing readback for {document_id}")
        revision = readback["metadata"].get("revision")
        if revision != item.get("revision"):
            raise WritebackError("WORKSPACE_READBACK_INVALID", f"revision mismatch for {document_id}")
        if item.get("revision") is None or item.get("deleted"):
            raise WritebackError("WORKSPACE_READBACK_INVALID", f"receipt revision is invalid for {document_id}")
        if item.get("previousRevision") is not None and item.get("previousRevision") >= expected_revision + 1:
            raise WritebackError("WORKSPACE_RECEIPT_INVALID", f"previous revision is invalid for {document_id}")


def _request_base(request_id: str, document_ids: list[str], arguments: dict[str, Any]) -> dict[str, Any]:
    return {
        "schemaVersion": 1,
        "requestId": request_id,
        "toolCallId": f"call-{request_id}",
        "scope": {"documentIds": document_ids},
        "arguments": arguments,
    }


def _writeback_partition(
    config: WritebackConfig,
    workspace_root: Path,
    document_id: str,
    prediction: Mapping[str, Any],
    initial: Mapping[str, Any],
    request_id: str,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    mode = prediction.get("mode")
    if mode not in {"partition", "extract"}:
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "semantic writeback requires partition or extract mode")
    arguments = {
        "parentDocumentId": document_id,
        "expectedParentRevision": initial["metadata"]["revision"],
        "sourceMarkdownSha256": initial["metadata"]["markdownSha256"],
        "mode": mode,
        "children": _semantic_children(prediction),
    }
    response = _invoke(
        config,
        workspace_root,
        document_id,
        "create_semantic_children",
        _request_base(request_id, [document_id], arguments),
    )
    receipt = _receipt(response, "create_semantic_children")
    readbacks = [_read_document(config, workspace_root, child_id, f"{request_id}-read-{index}") for index, child_id in enumerate(_expected_document_ids(receipt))]
    _verify_receipt_readback(
        receipt,
        readbacks,
        expected_revision=int(initial["metadata"]["revision"]),
        expected_created_count=len(arguments["children"]) + 1,
    )
    return receipt, readbacks


def _mutation_patch(mutation: Mapping[str, Any]) -> tuple[str | None, dict[str, Any]]:
    typed_operation = mutation.get("op")
    if typed_operation is not None and typed_operation != "update_document":
        typed_fields = {
            "set_title": "title",
            "set_tags": "tags",
            "set_summary": "summary",
            "set_markdown": "markdown",
            "set_document_status": "documentStatus",
        }
        field = typed_fields.get(typed_operation) if isinstance(typed_operation, str) else None
        if field is None:
            raise WritebackError(
                "PREDICTION_ARGUMENTS_INVALID",
                f"unsupported typed organize operation: {typed_operation}",
            )
        unknown = set(mutation) - {
            "op",
            "value",
            "documentId",
            "evidence_segment_ids",
            "evidenceSegmentIds",
            "rationale",
            "semanticRationale",
            "description",
            "explanation",
        }
        if unknown or "value" not in mutation:
            detail = f" contains unsupported fields: {sorted(unknown)}" if unknown else " lacks value"
            raise WritebackError(
                "PREDICTION_ARGUMENTS_INVALID",
                f"typed organize mutation {typed_operation}{detail}",
            )
        evidence_key = "evidence_segment_ids" if "evidence_segment_ids" in mutation else "evidenceSegmentIds"
        if evidence_key in mutation:
            evidence = mutation[evidence_key]
            if (
                not isinstance(evidence, list)
                or not evidence
                or not all(isinstance(item, str) and item.strip() for item in evidence)
            ):
                raise WritebackError(
                    "PREDICTION_ARGUMENTS_INVALID",
                    "evidence_segment_ids must be a non-empty string array",
                )
        value = mutation["value"]
        if field == "title":
            if not isinstance(value, str) or not value.strip():
                raise WritebackError(
                    "PREDICTION_ARGUMENTS_INVALID",
                    "set_title value must be a non-empty string",
                )
        elif field == "tags":
            value = _tags(value)
        elif field == "summary" and value is not None and not isinstance(value, str):
            raise WritebackError(
                "PREDICTION_ARGUMENTS_INVALID",
                "set_summary value must be a string or null",
                )
        elif field in {"markdown", "documentStatus"} and not isinstance(value, str):
            raise WritebackError(
                "PREDICTION_ARGUMENTS_INVALID",
                f"{typed_operation} value must be a string",
            )
        return mutation.get("documentId") if isinstance(mutation.get("documentId"), str) else None, {field: value}

    range_fields = {
        "start_segment_id",
        "end_segment_id",
        "uncovered_ranges",
        "children",
        "spans",
        "segmentIds",
        "blockId",
        "mode",
        "parent_child_index",
        "second_pass",
        "secondPass",
        "uncoveredSpans",
    }
    if set(mutation) & range_fields:
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "organize mutation contains range fields")
    operation = mutation.get("operation", mutation.get("tool", mutation.get("op")))
    if operation not in (None, "update_document"):
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"unsupported organize operation: {operation}")
    document_id = mutation.get("documentId")
    patch = mutation.get("patch", mutation.get("changes", mutation.get("fields")))
    if patch is None and operation == "update_document":
        patch = mutation.get("value")
    if not isinstance(patch, Mapping) or not patch:
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "organize mutation patch is missing")
    allowed = {"title", "tags", "summary", "markdown", "documentStatus"}
    if "document_status" in patch and "documentStatus" in patch:
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "organize patch supplies both document_status and documentStatus")
    normalized_patch = dict(patch)
    if "document_status" in normalized_patch:
        normalized_patch["documentStatus"] = normalized_patch.pop("document_status")
    unknown = set(normalized_patch) - allowed
    if unknown:
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"organize patch contains unsupported fields: {sorted(unknown)}")
    normalized = normalized_patch
    if "markdown" in normalized:
        if not isinstance(normalized["markdown"], str):
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "organize markdown patch must be a string")
        if normalized["markdown"] and not normalized["markdown"].endswith("\n"):
            normalized["markdown"] += "\n"
    return document_id if isinstance(document_id, str) else None, normalized


def _writeback_organize(
    config: WritebackConfig,
    workspace_root: Path,
    document_id: str,
    prediction: Mapping[str, Any],
    initial: Mapping[str, Any],
    request_id: str,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    mutations = prediction.get("mutations")
    if not isinstance(mutations, list) or not mutations:
        raise WritebackError("PREDICTION_ARGUMENTS_INVALID", "organize prediction mutations are empty")
    current = dict(initial)
    receipts: list[dict[str, Any]] = []
    readbacks: list[dict[str, Any]] = []
    for index, raw_mutation in enumerate(mutations):
        if not isinstance(raw_mutation, Mapping):
            raise WritebackError("PREDICTION_ARGUMENTS_INVALID", f"organize mutation {index} is not an object")
        target_id, patch = _mutation_patch(raw_mutation)
        if target_id not in (None, document_id):
            raise WritebackError("PREDICTION_SCOPE_VIOLATION", "organize mutation targets a different document")
        current_metadata = current["metadata"]
        response = _invoke(
            config,
            workspace_root,
            document_id,
            "update_document",
            _request_base(
                f"{request_id}-{index}",
                [document_id],
                {
                    "documentId": document_id,
                    "expectedRevision": current_metadata["revision"],
                    "sourceMarkdownSha256": current_metadata["markdownSha256"],
                    "patch": patch,
                },
            ),
        )
        receipt = _receipt(response, "update_document")
        current_readback = _read_document(config, workspace_root, document_id, f"{request_id}-read-{index}")
        _verify_receipt_readback(
            receipt,
            [current_readback],
            expected_revision=int(current_metadata["revision"]),
            expected_created_count=1,
        )
        receipts.append(receipt)
        readbacks.append(current_readback)
        current = current_readback
    return receipts, readbacks


def writeback_prediction(
    prediction: Mapping[str, Any],
    record: Mapping[str, Any],
    config: WritebackConfig,
    *,
    family: str,
    run_key: str,
) -> dict[str, Any]:
    """Apply one external prediction and return independently verified evidence."""

    if family not in WRITEBACK_FAMILIES:
        raise WritebackError("WRITEBACK_FAMILY_UNSUPPORTED", f"writeback is not enabled for family {family}")
    if not isinstance(prediction, Mapping) or not isinstance(record, Mapping):
        raise WritebackError("WRITEBACK_INPUT_INVALID", "prediction and record must be objects")
    _reject_evaluator_prediction_fields(prediction)
    context, workspace_root, document_id = create_temporary_document_package(record, config.package_root, run_key)
    try:
        initial = _read_document(config, workspace_root, document_id, f"{run_key}-initial")
        if family == "partition":
            receipt, readbacks = _writeback_partition(
                config, workspace_root, document_id, prediction, initial, f"{run_key}-partition"
            )
            receipts = [receipt]
        else:
            receipts, readbacks = _writeback_organize(
                config, workspace_root, document_id, prediction, initial, f"{run_key}-organize"
            )
        return {
            "scenarioId": prediction.get("scenarioId"),
            "documentId": document_id,
            "mutationReceipts": receipts,
            "readback": readbacks,
            "readbackSha256": _sha256_bytes(
                json.dumps(readbacks, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
            ),
        }
    except WritebackError as error:
        raise WritebackError(
            error.code,
            error.reason,
            receipts=error.receipts,
            readback=error.readback,
        ) from error
    finally:
        if config.keep_packages:
            context.cleanup = lambda: None  # type: ignore[method-assign]
        else:
            context.cleanup()


def writeback_predictions(
    predictions: Sequence[Mapping[str, Any]],
    records: Sequence[Mapping[str, Any]],
    config: WritebackConfig,
    *,
    family: str,
    run_key: str,
) -> dict[str, Any]:
    by_id = {record.get("scenarioId"): record for record in records}
    evidence: list[dict[str, Any]] = []
    receipts: list[dict[str, Any]] = []
    readbacks: list[dict[str, Any]] = []
    try:
        for index, prediction in enumerate(predictions):
            scenario_id = prediction.get("scenarioId") if isinstance(prediction, Mapping) else None
            record = by_id.get(scenario_id)
            if not isinstance(record, Mapping):
                raise WritebackError("WRITEBACK_RECORD_NOT_FOUND", f"no public record for {scenario_id}")
            item = writeback_prediction(
                prediction,
                record,
                config,
                family=family,
                run_key=f"{run_key}-{index}",
            )
            evidence.append(item)
            receipts.extend(item["mutationReceipts"])
            readbacks.extend(item["readback"])
    except WritebackError as error:
        raise WritebackError(error.code, error.reason, receipts=receipts, readback=readbacks) from error
    return {
        "mutationReceipts": receipts,
        "readback": readbacks,
        "evidence": evidence,
    }
