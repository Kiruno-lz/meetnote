#!/usr/bin/env python3
"""A small, real HTTP runner for Assistant semantic evaluation.

The runner is intentionally stateless.  It accepts one public request on
stdin, sends only that request to the configured model endpoint, and emits one
JSON object on stdout.  It never loads the corpus, fixtures, or files, so gold
labels and evaluator-only negative cases cannot become an implicit oracle.

Configuration accepts the names used by the Rust real-upstream tests as well
as protocol-specific Assistant-eval names.  The most specific name wins; for
example ``MEETNOTE_REAL_LLM_OPENAI_BASE_URL`` is used by both OpenAI protocol
adapters while ``MEETNOTE_ASSISTANT_EVAL_OPENAI_RESPONSES_ENDPOINT`` is an
exact endpoint override for the Responses adapter.
"""

from __future__ import annotations

import copy
import json
import os
import re
import sys
from dataclasses import dataclass
from http.client import HTTPResponse
from typing import Any, Mapping
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit, urlunsplit
from urllib.request import Request, urlopen

from evaluate import validate_structure


PROTOCOLS = {
    "openaiChatCompletions": {
        "path": "/v1/chat/completions",
        "endpoint": (
            "MEETNOTE_ASSISTANT_EVAL_OPENAICHATCOMPLETIONS_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_CHAT_COMPLETIONS_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_CHAT_COMPLETIONS_URL",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_CHAT_COMPLETIONS_BASE_URL",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_BASE_URL",
            "MEETNOTE_ASSISTANT_EVAL_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_BASE_URL",
            "MEETNOTE_REAL_LLM_OPENAI_ENDPOINT",
            "MEETNOTE_REAL_LLM_OPENAI_BASE_URL",
            "MEETNOTE_REAL_LLM_BASE_URL",
        ),
        "model": (
            "MEETNOTE_ASSISTANT_EVAL_OPENAICHATCOMPLETIONS_MODEL",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_CHAT_COMPLETIONS_MODEL",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_MODEL",
            "MEETNOTE_ASSISTANT_EVAL_MODEL",
            "MEETNOTE_REAL_LLM_OPENAI_MODEL",
            "MEETNOTE_REAL_LLM_MODEL",
        ),
        "key": (
            "MEETNOTE_ASSISTANT_EVAL_OPENAICHATCOMPLETIONS_API_KEY",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_CHAT_COMPLETIONS_API_KEY",
            "MEETNOTE_ASSISTANT_EVAL_OPENAICHATCOMPLETIONS_KEY",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_CHAT_COMPLETIONS_KEY",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_KEY",
            "MEETNOTE_ASSISTANT_EVAL_API_KEY",
            "MEETNOTE_REAL_LLM_OPENAI_API_KEY",
            "MEETNOTE_REAL_LLM_OPENAI_KEY",
            "MEETNOTE_REAL_LLM_API_KEY",
            "OPENAI_API_KEY",
        ),
        "auth": (
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_AUTH_MODE",
            "MEETNOTE_REAL_LLM_OPENAI_AUTH_MODE",
            "MEETNOTE_ASSISTANT_EVAL_AUTH_MODE",
            "MEETNOTE_REAL_LLM_AUTH_MODE",
        ),
    },
    "anthropicMessages": {
        "path": "/v1/messages",
        "endpoint": (
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPICMESSAGES_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_MESSAGES_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_MESSAGES_URL",
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_MESSAGES_BASE_URL",
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_BASE_URL",
            "MEETNOTE_ASSISTANT_EVAL_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_BASE_URL",
            "MEETNOTE_REAL_LLM_ANTHROPIC_ENDPOINT",
            "MEETNOTE_REAL_LLM_ANTHROPIC_BASE_URL",
            "MEETNOTE_REAL_LLM_BASE_URL",
        ),
        "model": (
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPICMESSAGES_MODEL",
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_MESSAGES_MODEL",
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_MODEL",
            "MEETNOTE_ASSISTANT_EVAL_MODEL",
            "MEETNOTE_REAL_LLM_ANTHROPIC_MODEL",
            "MEETNOTE_REAL_LLM_MODEL",
        ),
        "key": (
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPICMESSAGES_API_KEY",
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_MESSAGES_API_KEY",
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPICMESSAGES_KEY",
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_MESSAGES_KEY",
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_KEY",
            "MEETNOTE_ASSISTANT_EVAL_API_KEY",
            "MEETNOTE_REAL_LLM_ANTHROPIC_API_KEY",
            "MEETNOTE_REAL_LLM_ANTHROPIC_KEY",
            "MEETNOTE_REAL_LLM_API_KEY",
            "ANTHROPIC_API_KEY",
        ),
        "auth": (
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_AUTH_MODE",
            "MEETNOTE_REAL_LLM_ANTHROPIC_AUTH_MODE",
            "MEETNOTE_ASSISTANT_EVAL_AUTH_MODE",
            "MEETNOTE_REAL_LLM_AUTH_MODE",
        ),
    },
    "openaiResponses": {
        "path": "/v1/responses",
        "endpoint": (
            "MEETNOTE_ASSISTANT_EVAL_OPENAIRESPONSES_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_RESPONSES_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_RESPONSES_URL",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_RESPONSES_BASE_URL",
            "MEETNOTE_ASSISTANT_EVAL_RESPONSES_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_RESPONSES_BASE_URL",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_BASE_URL",
            "MEETNOTE_ASSISTANT_EVAL_ENDPOINT",
            "MEETNOTE_ASSISTANT_EVAL_BASE_URL",
            "MEETNOTE_REAL_LLM_RESPONSES_ENDPOINT",
            "MEETNOTE_REAL_LLM_RESPONSES_BASE_URL",
            "MEETNOTE_REAL_LLM_BASE_URL",
        ),
        "model": (
            "MEETNOTE_ASSISTANT_EVAL_OPENAIRESPONSES_MODEL",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_RESPONSES_MODEL",
            "MEETNOTE_ASSISTANT_EVAL_RESPONSES_MODEL",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_MODEL",
            "MEETNOTE_ASSISTANT_EVAL_MODEL",
            "MEETNOTE_REAL_LLM_RESPONSES_MODEL",
            "MEETNOTE_REAL_LLM_MODEL",
        ),
        "key": (
            "MEETNOTE_ASSISTANT_EVAL_OPENAIRESPONSES_API_KEY",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_RESPONSES_API_KEY",
            "MEETNOTE_ASSISTANT_EVAL_OPENAIRESPONSES_KEY",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_RESPONSES_KEY",
            "MEETNOTE_ASSISTANT_EVAL_RESPONSES_KEY",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_KEY",
            "MEETNOTE_ASSISTANT_EVAL_API_KEY",
            "MEETNOTE_REAL_LLM_RESPONSES_API_KEY",
            "MEETNOTE_REAL_LLM_RESPONSES_KEY",
            "MEETNOTE_REAL_LLM_API_KEY",
            "OPENAI_API_KEY",
        ),
        "auth": (
            "MEETNOTE_ASSISTANT_EVAL_RESPONSES_AUTH_MODE",
            "MEETNOTE_REAL_LLM_RESPONSES_AUTH_MODE",
            "MEETNOTE_ASSISTANT_EVAL_AUTH_MODE",
            "MEETNOTE_REAL_LLM_AUTH_MODE",
        ),
    },
}

FORBIDDEN_KEYS = {
    "gold",
    "negativecase",
    "negativecases",
    "semanticrationale",
    "rationale",
}
ORACLE_MARKERS = re.compile(r"oracle|gold[-_ ]?copy|gold[-_ ]?prediction", re.IGNORECASE)
DEFAULT_TIMEOUT = 120.0
DEFAULT_MAX_INPUT_BYTES = 4 * 1024 * 1024
DEFAULT_MAX_RESPONSE_BYTES = 2 * 1024 * 1024
MAX_METADATA_BYTES = 512 * 1024


class RunnerError(ValueError):
    """A safe, structured runner failure."""

    def __init__(self, code: str, reason: str, *, status: str = "failed") -> None:
        super().__init__(reason)
        self.code = code
        self.reason = reason
        self.status = status


@dataclass(frozen=True)
class Configuration:
    protocol: str
    endpoint: str
    model: str
    api_key: str
    auth_mode: str


def _first_env(names: tuple[str, ...], environment: Mapping[str, str]) -> str | None:
    for name in names:
        value = environment.get(name, "").strip()
        if value:
            return value
    return None


def _protocol_config(protocol: str, environment: Mapping[str, str]) -> Configuration:
    spec = PROTOCOLS.get(protocol)
    if spec is None:
        raise RunnerError("UNSUPPORTED_PROTOCOL", f"unsupported protocol: {protocol}", status="not_assessed")
    endpoint = _first_env(spec["endpoint"], environment)
    model = _first_env(spec["model"], environment)
    api_key = _first_env(spec["key"], environment)
    missing = [name for name, value in (("endpoint", endpoint), ("model", model), ("api_key", api_key)) if not value]
    if missing:
        raise RunnerError(
            "MISSING_CONFIGURATION",
            f"missing {','.join(missing)} environment configuration for {protocol}",
            status="not_assessed",
        )
    auth = (_first_env(spec["auth"], environment) or ("x-api-key" if protocol == "anthropicMessages" else "bearer")).lower()
    if auth in {"xapikey", "x_api_key"}:
        auth = "x-api-key"
    if auth not in {"bearer", "x-api-key"}:
        raise RunnerError("INVALID_AUTH_MODE", f"unsupported auth mode for {protocol}", status="not_assessed")
    return Configuration(protocol, _join_endpoint(endpoint, spec["path"]), model, api_key, auth)


def _join_endpoint(value: str, path: str) -> str:
    parts = urlsplit(value)
    if parts.scheme not in {"http", "https"} or not parts.netloc or parts.username or parts.password:
        raise RunnerError("INVALID_ENDPOINT", "endpoint must be an http(s) URL without embedded credentials", status="not_assessed")
    current = parts.path.rstrip("/")
    wanted = path.rstrip("/")
    if current.endswith(wanted):
        final_path = current
    elif current.endswith("/v1") and wanted.startswith("/v1/"):
        final_path = current + wanted[3:]
    else:
        final_path = current + wanted
    return urlunsplit((parts.scheme, parts.netloc, final_path, parts.query, ""))


def _contains_forbidden_key(value: Any, path: str = "$") -> str | None:
    if isinstance(value, dict):
        for key, child in value.items():
            if not isinstance(key, str):
                return f"{path} contains a non-string key"
            if key.lower() in FORBIDDEN_KEYS:
                return f"{path}.{key} is evaluator-only input"
            found = _contains_forbidden_key(child, f"{path}.{key}")
            if found:
                return found
    elif isinstance(value, list):
        for index, child in enumerate(value):
            found = _contains_forbidden_key(child, f"{path}[{index}]")
            if found:
                return found
    return None


def _contains_oracle_key(value: Any, path: str = "$") -> str | None:
    """Check model output for evaluator labels, while allowing model rationale.

    ``semanticRationale`` is a model-produced field used by the ``none``
    contract.  It is forbidden in the input projection, but it is not itself
    gold evidence when returned by the model.  Gold and negative-case fields
    remain forbidden on output.
    """
    if isinstance(value, dict):
        for key, child in value.items():
            if isinstance(key, str) and key.lower() in {"gold", "negativecase", "negativecases"}:
                return f"{path}.{key} is evaluator-only output"
            found = _contains_oracle_key(child, f"{path}.{key}")
            if found:
                return found
    elif isinstance(value, list):
        for index, child in enumerate(value):
            found = _contains_oracle_key(child, f"{path}[{index}]")
            if found:
                return found
    return None


def validate_public_request(request: Any) -> dict[str, Any]:
    if not isinstance(request, dict):
        raise RunnerError("INVALID_REQUEST", "stdin JSON must be an object", status="not_assessed")
    try:
        encoded_request = json.dumps(request, ensure_ascii=False).encode("utf-8")
    except (TypeError, ValueError) as error:
        raise RunnerError("INVALID_REQUEST", "request cannot be encoded as JSON", status="not_assessed") from error
    resources = request.get("resources", {})
    max_input = DEFAULT_MAX_INPUT_BYTES
    if isinstance(resources, dict) and "maxInputBytes" in resources:
        try:
            max_input = int(resources["maxInputBytes"])
        except (TypeError, ValueError) as error:
            raise RunnerError("INVALID_REQUEST", "resources.maxInputBytes must be an integer", status="not_assessed") from error
    if max_input <= 0 or len(encoded_request) > max_input:
        raise RunnerError("INPUT_SIZE_EXCEEDED", "request exceeds the configured byte limit", status="not_assessed")
    forbidden = _contains_forbidden_key(request)
    if forbidden:
        raise RunnerError("EVALUATOR_DATA_IN_REQUEST", forbidden, status="not_assessed")
    if request.get("schemaVersion") != 1:
        raise RunnerError("INVALID_REQUEST_SCHEMA", "request schemaVersion must be 1", status="not_assessed")
    holdout = request.get("holdout")
    if isinstance(holdout, dict) and holdout.get("goldIncluded") is not False:
        raise RunnerError(
            "EVALUATOR_DATA_IN_REQUEST",
            "holdout.goldIncluded must be false",
            status="not_assessed",
        )
    protocol = request.get("protocol")
    if protocol not in PROTOCOLS:
        raise RunnerError("UNSUPPORTED_PROTOCOL", f"unsupported protocol: {protocol!r}", status="not_assessed")
    records = request.get("records")
    if not isinstance(records, list) or not records:
        raise RunnerError("INVALID_REQUEST", "request records must be a non-empty array", status="not_assessed")
    if any(not isinstance(record, dict) or not isinstance(record.get("scenarioId"), str) for record in records):
        raise RunnerError("INVALID_REQUEST", "every record must have a scenarioId", status="not_assessed")
    return request


def _public_prompt(request: dict[str, Any]) -> str:
    contract = request.get("outputContract")
    contract_text = json.dumps(contract, ensure_ascii=False, sort_keys=True)
    family = request.get("taskFamily")
    family_contract = ""
    if family == "validator":
        family_contract = (
            " For validator tasks, return a correction object with a non-empty "
            "errorCode, an object correctedOperation, and retry exactly false; "
            "never retry blindly or omit any of these fields."
        )
    elif family == "organize":
        family_contract = (
            " For organize tasks, return at least one typed mutation. Use only "
            "set_title, set_tags, set_summary, or update_document with a non-empty "
            "patch; never return range/partition/child fields as a mutation."
        )
    public = {
        "caseId": request.get("caseId"),
        "taskFamily": request.get("taskFamily"),
        "prompt": request.get("prompt"),
        "records": request.get("records"),
        "semanticFixtures": request.get("semanticFixtures", []),
        "outputContract": contract,
    }
    return (
        "Return only one JSON object with a non-empty `predictions` array. "
        "Do not return markdown, evaluator labels, gold data, negative cases, or an oracle. "
        "Each prediction must use the exact scenarioId from the supplied records and satisfy "
        f"the output contract {contract_text}. Preserve only actual tool/readback/receipt "
        "metadata in a top-level `metadata` object; do not invent receipts. "
        "Keep each answer, title, summary, and rationale concise (at most 60 words); "
        "For ask tasks, citations MUST be a non-empty JSON array of exact supplied segment IDs; "
        "never use an empty array and never invent an ID; when evidence is insufficient, cite "
        "the supplied segments that demonstrate the insufficiency. "
        "return no commentary outside the JSON value. "
        "For range operations use snake_case fields: start_segment_id, end_segment_id, "
        "evidence_segment_ids, uncovered_ranges; nested second_pass uses parent_child_index "
        "and preserves the selected parent's complete range."
        + family_contract
        + "\n\n"
        + json.dumps(public, ensure_ascii=False, sort_keys=True)
    )


def _json_format() -> dict[str, Any]:
    return {"type": "json_object"}


def build_payload(request: dict[str, Any], config: Configuration) -> dict[str, Any]:
    system = request["prompt"].get("system", "") if isinstance(request.get("prompt"), dict) else ""
    user = _public_prompt(request)
    max_tokens = int(request.get("budget", {}).get("maxOutputTokens", 1024))
    disable_thinking = os.environ.get("MEETNOTE_ASSISTANT_EVAL_DISABLE_THINKING") == "1"
    if config.protocol == "openaiChatCompletions":
        payload = {
            "model": config.model,
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
            "temperature": 0,
            "max_tokens": max_tokens,
            "stream": False,
            "response_format": _json_format(),
        }
        if disable_thinking:
            payload["thinking"] = {"type": "disabled"}
        return payload
    if config.protocol == "anthropicMessages":
        payload = {
            "model": config.model,
            "system": system,
            "messages": [{"role": "user", "content": user}],
            "temperature": 0,
            "max_tokens": max_tokens,
            "stream": False,
        }
        if disable_thinking:
            payload["thinking"] = {"type": "disabled"}
        return payload
    payload = {
        "model": config.model,
        "instructions": system,
        "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": user}]}],
        "temperature": 0,
        "max_output_tokens": max_tokens,
        "store": False,
        "stream": False,
        "text": {"format": _json_format()},
    }
    if disable_thinking:
        payload["reasoning"] = {"effort": "none"}
        provider_hint = os.environ.get("MEETNOTE_ASSISTANT_EVAL_RESPONSES_PROVIDER", "").strip().lower()
        is_deepseek = provider_hint == "deepseek" or "deepseek" in config.model.lower()
        if is_deepseek:
            # DeepSeek-compatible OpenAI gateways may ignore the Responses
            # reasoning object.  These aliases are provider hints only; the
            # response is still required to be a completed structured result.
            payload["thinking"] = {"type": "disabled"}
            payload["reasoning_effort"] = "none"
    return payload


def _read_limited(response: HTTPResponse, limit: int) -> bytes:
    body = response.read(limit + 1)
    if len(body) > limit:
        raise RunnerError("HTTP_RESPONSE_TOO_LARGE", "model response exceeds the configured byte limit")
    return body


def _http_json(config: Configuration, payload: dict[str, Any], timeout: float, max_bytes: int) -> dict[str, Any]:
    headers = {"Content-Type": "application/json", "Accept": "application/json"}
    if config.auth_mode == "x-api-key":
        headers["x-api-key"] = config.api_key
    else:
        headers["Authorization"] = f"Bearer {config.api_key}"
    if config.protocol == "anthropicMessages":
        headers["anthropic-version"] = "2023-06-01"
    request = Request(config.endpoint, data=json.dumps(payload, ensure_ascii=False).encode("utf-8"), headers=headers, method="POST")
    try:
        with urlopen(request, timeout=timeout) as response:
            body = _read_limited(response, max_bytes)
            status = getattr(response, "status", 200)
    except HTTPError as error:
        # Do not echo the provider body: gateway errors occasionally contain
        # request headers, endpoint fragments, or other credential-adjacent
        # data.  The HTTP status is sufficient for a structured failure.
        try:
            error.close()
        except OSError:
            pass
        raise RunnerError("HTTP_ERROR", f"model endpoint returned HTTP {error.code}") from error
    except (URLError, TimeoutError, OSError) as error:
        reason = "request timed out" if isinstance(error, TimeoutError) else "model endpoint is unreachable"
        raise RunnerError("HTTP_TRANSPORT_ERROR", reason) from error
    if status < 200 or status >= 300:
        raise RunnerError("HTTP_ERROR", f"model endpoint returned HTTP {status}")
    try:
        result = json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RunnerError("HTTP_INVALID_JSON", "model endpoint returned invalid JSON") from error
    if not isinstance(result, dict):
        raise RunnerError("HTTP_INVALID_JSON", "model endpoint JSON root must be an object")
    if isinstance(result.get("error"), dict):
        raise RunnerError("MODEL_ERROR", "model endpoint returned an error")
    return result


def _text_parts(response: dict[str, Any], protocol: str) -> tuple[str, list[dict[str, Any]]]:
    tool_calls: list[dict[str, Any]] = []
    texts: list[str] = []
    if protocol == "openaiChatCompletions":
        choices = response.get("choices")
        if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
            raise RunnerError("MODEL_OUTPUT_INVALID", "chat response has no choice")
        message = choices[0].get("message")
        if not isinstance(message, dict):
            raise RunnerError("MODEL_OUTPUT_INVALID", "chat response has no message")
        content = message.get("content")
        if isinstance(content, str):
            texts.append(content)
        elif isinstance(content, list):
            for item in content:
                if isinstance(item, dict) and isinstance(item.get("text"), str):
                    texts.append(item["text"])
        for call in message.get("tool_calls", []) if isinstance(message.get("tool_calls", []), list) else []:
            if isinstance(call, dict):
                tool_calls.append(_safe_metadata(call))
    elif protocol == "anthropicMessages":
        content = response.get("content")
        if not isinstance(content, list):
            raise RunnerError("MODEL_OUTPUT_INVALID", "Anthropic response has no content array")
        for item in content:
            if not isinstance(item, dict):
                continue
            if item.get("type") == "text" and isinstance(item.get("text"), str):
                texts.append(item["text"])
            elif item.get("type") == "tool_use":
                tool_calls.append(_safe_metadata(item))
    else:
        output = response.get("output")
        if not isinstance(output, list):
            if isinstance(response.get("output_text"), str):
                return response["output_text"], tool_calls
            raise RunnerError("MODEL_OUTPUT_INVALID", "Responses response has no output array")
        for item in output:
            if not isinstance(item, dict):
                continue
            if item.get("type") == "message":
                for part in item.get("content", []) if isinstance(item.get("content", []), list) else []:
                    if isinstance(part, dict) and part.get("type") == "output_text" and isinstance(part.get("text"), str):
                        texts.append(part["text"])
            elif item.get("type") in {"function_call", "tool_call"}:
                tool_calls.append(_safe_metadata(item))
        if isinstance(response.get("output_text"), str) and not texts:
            texts.append(response["output_text"])
    return "\n".join(texts), tool_calls


def _decode_json_text(text: str) -> Any:
    value = text.strip()
    if value.startswith("```"):
        lines = value.splitlines()
        if lines and lines[0].strip().startswith("```"):
            lines = lines[1:]
        if lines and lines[-1].strip() == "```":
            lines = lines[:-1]
        value = "\n".join(lines).strip()
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError as error:
        raise RunnerError("MODEL_OUTPUT_INVALID", f"model output is not one JSON value: {error.msg}") from error
    if not isinstance(parsed, (dict, list)):
        raise RunnerError("MODEL_OUTPUT_INVALID", "model output JSON must be an object or array")
    return parsed


def _safe_metadata(value: Any) -> Any:
    if isinstance(value, dict):
        result: dict[str, Any] = {}
        for key, child in value.items():
            if not isinstance(key, str) or len(key) > 128:
                continue
            lowered = key.lower()
            if any(marker in lowered for marker in ("api_key", "apikey", "authorization", "password", "secret", "token")):
                result[key] = "[redacted]"
            else:
                result[key] = _safe_metadata(child)
        return result
    if isinstance(value, list):
        return [_safe_metadata(item) for item in value[:64]]
    if isinstance(value, str):
        value = re.sub(r"(?i)\b(?:bearer\s+)?sk-[a-z0-9_-]{8,}\b", "[redacted]", value)
        return value[:8192]
    if isinstance(value, (bool, int, float)) or value is None:
        return value
    return str(value)[:256]


def _extract_result(response: dict[str, Any], protocol: str) -> tuple[Any, list[dict[str, Any]]]:
    if protocol == "openaiResponses" and response.get("status") == "incomplete":
        reason = response.get("incomplete_details", {}).get("reason") if isinstance(response.get("incomplete_details"), dict) else None
        raise RunnerError(
            "MODEL_INCOMPLETE",
            f"Responses model returned incomplete status: {reason or 'unknown reason'}",
        )
    text, tool_calls = _text_parts(response, protocol)
    if not text.strip():
        if tool_calls:
            raise RunnerError("MODEL_TOOL_CALL_ONLY", "model returned a tool call without a final structured prediction")
        raise RunnerError("MODEL_OUTPUT_INVALID", "model returned no structured text")
    parsed = _decode_json_text(text)
    oracle_key = _contains_oracle_key(parsed)
    if oracle_key:
        raise RunnerError("ORACLE_OUTPUT_REJECTED", oracle_key)
    serialized = json.dumps(parsed, ensure_ascii=False, sort_keys=True)
    if ORACLE_MARKERS.search(serialized):
        raise RunnerError("ORACLE_OUTPUT_REJECTED", "model output contains an oracle marker")
    return parsed, tool_calls


def _prediction_list(parsed: Any) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    metadata: dict[str, Any] = {}
    if isinstance(parsed, list):
        predictions = parsed
    elif isinstance(parsed, dict) and isinstance(parsed.get("predictions"), list):
        predictions = parsed["predictions"]
        if isinstance(parsed.get("metadata"), dict):
            metadata = _safe_metadata(parsed["metadata"])
        for field in ("toolCalls", "readback", "readbacks", "mutationReceipts", "receipt", "receipts", "receiptAvailable"):
            if field in parsed and field not in metadata:
                metadata[field] = _safe_metadata(parsed[field])
    elif isinstance(parsed, dict) and isinstance(parsed.get("prediction"), dict):
        predictions = [parsed["prediction"]]
        if isinstance(parsed.get("metadata"), dict):
            metadata = _safe_metadata(parsed["metadata"])
        for field in ("toolCalls", "readback", "readbacks", "mutationReceipts", "receipt", "receipts", "receiptAvailable"):
            if field in parsed and field not in metadata:
                metadata[field] = _safe_metadata(parsed[field])
    elif isinstance(parsed, dict) and isinstance(parsed.get("scenarioId"), str):
        predictions = [parsed]
    else:
        raise RunnerError("MODEL_OUTPUT_INVALID", "structured output must contain predictions")
    if not predictions or not all(isinstance(item, dict) for item in predictions):
        raise RunnerError("MODEL_OUTPUT_INVALID", "predictions must be a non-empty array of objects")
    return predictions, metadata


def _supplied_segment_ids(request: dict[str, Any], scenario_id: str) -> set[str]:
    for record in request.get("records", []):
        if not isinstance(record, dict) or record.get("scenarioId") != scenario_id:
            continue
        payload = record.get("record")
        state = payload.get("state") if isinstance(payload, dict) else None
        results = state.get("results") if isinstance(state, dict) else None
        transcript = results.get("transcript") if isinstance(results, dict) else None
        if isinstance(transcript, list):
            return {
                segment.get("id")
                for segment in transcript
                if isinstance(segment, dict) and isinstance(segment.get("id"), str) and segment["id"]
            }
    return set()


def _validate_semantic_structure(request: dict[str, Any], prediction: dict[str, Any]) -> None:
    """Validate standard Workspace range predictions without reading gold data."""
    family = request.get("taskFamily")
    if family not in {"partition", "extract", "none", "nested"}:
        return
    records = request.get("records")
    if not isinstance(records, list):
        return
    record = next(
        (item for item in records if isinstance(item, dict) and item.get("scenarioId") == prediction.get("scenarioId")),
        None,
    )
    if not isinstance(record, dict):
        return
    transcript = record.get("record", {}).get("state", {}).get("results", {}).get("transcript")
    if isinstance(transcript, list):
        segment_ids = [item.get("id") for item in transcript if isinstance(item, dict)]
    else:
        segments = record.get("segments")
        segment_ids = [item.get("id") for item in segments if isinstance(item, dict)] if isinstance(segments, list) else []
    if not segment_ids or not all(isinstance(item, str) and item for item in segment_ids):
        return
    children = prediction.get("children")
    # Cross-scope public predictions use block spans and are validated by their
    # dedicated evaluator; standard Workspace predictions use snake_case ranges.
    if isinstance(children, list) and children and not all(
        isinstance(child, dict) and "start_segment_id" in child and "end_segment_id" in child
        for child in children
    ):
        return
    validation = validate_structure(
        segment_ids,
        prediction.get("mode"),
        children,
        prediction.get("uncovered_ranges"),
        prediction.get("second_pass"),
    )
    if not validation.valid:
        raise RunnerError(
            "PREDICTION_CONTRACT_INVALID",
            f"{family} semantic structure is invalid: {'; '.join(validation.errors)}",
        )


def _validate_typed_organize_mutation(mutation: Any, index: int) -> None:
    if not isinstance(mutation, dict):
        raise RunnerError("PREDICTION_CONTRACT_INVALID", f"organize mutation {index} must be an object")
    if "op" in mutation and mutation.get("op") != "update_document":
        allowed_ops = {"set_title", "set_tags", "set_summary", "set_markdown", "set_document_status"}
        if mutation.get("op") not in allowed_ops:
            raise RunnerError(
                "PREDICTION_CONTRACT_INVALID",
                f"organize mutation {index} has unsupported typed op: {mutation.get('op')!r}",
            )
        allowed_keys = {
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
        unknown = set(mutation) - allowed_keys
        if unknown or "value" not in mutation:
            raise RunnerError("PREDICTION_CONTRACT_INVALID", f"organize mutation {index} must contain only typed op and value")
        evidence = mutation.get("evidence_segment_ids", mutation.get("evidenceSegmentIds"))
        if evidence is not None and (
            not isinstance(evidence, list) or not evidence or not all(isinstance(item, str) and item for item in evidence)
        ):
            raise RunnerError("PREDICTION_CONTRACT_INVALID", f"organize mutation {index} evidence_segment_ids must be non-empty strings")
        return
    operation = mutation.get("operation", mutation.get("tool", mutation.get("op")))
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
        raise RunnerError("PREDICTION_CONTRACT_INVALID", f"organize mutation {index} contains range fields")
    if operation is None and not any(key in mutation for key in ("patch", "changes", "fields")):
        raise RunnerError("PREDICTION_CONTRACT_INVALID", f"organize mutation {index} requires a typed op or non-empty patch")
    if operation not in (None, "update_document"):
        raise RunnerError("PREDICTION_CONTRACT_INVALID", f"organize mutation {index} has unsupported operation")
    patch = mutation.get("patch", mutation.get("changes", mutation.get("fields")))
    if patch is None and operation == "update_document":
        patch = mutation.get("value")
    if not isinstance(patch, dict) or not patch:
        raise RunnerError("PREDICTION_CONTRACT_INVALID", f"organize mutation {index} requires a non-empty patch")
    if "document_status" in patch and "documentStatus" in patch:
        raise RunnerError("PREDICTION_CONTRACT_INVALID", f"organize mutation {index} supplies both document_status and documentStatus")
    unknown_patch = set(patch) - {"title", "tags", "summary", "markdown", "documentStatus", "document_status"}
    if unknown_patch:
        raise RunnerError("PREDICTION_CONTRACT_INVALID", f"organize mutation {index} patch has unsupported fields: {sorted(unknown_patch)}")


def _validate_predictions(request: dict[str, Any], predictions: list[dict[str, Any]]) -> None:
    expected = [record["scenarioId"] for record in request["records"]]
    actual = [item.get("scenarioId") for item in predictions]
    if any(not isinstance(value, str) for value in actual) or sorted(actual) != sorted(expected) or len(set(actual)) != len(actual):
        raise RunnerError("PREDICTION_COVERAGE_INVALID", "predictions must cover each supplied scenarioId exactly once")
    family = request.get("taskFamily")
    required = request.get("outputContract", {}).get("required", []) if isinstance(request.get("outputContract"), dict) else []
    if not isinstance(required, list):
        raise RunnerError("REQUEST_CONTRACT_INVALID", "outputContract.required must be an array", status="not_assessed")
    for prediction in predictions:
        forbidden = _contains_oracle_key(prediction)
        if forbidden:
            raise RunnerError("ORACLE_OUTPUT_REJECTED", forbidden)
        missing = [key for key in required if key not in prediction]
        if missing:
            raise RunnerError(
                "PREDICTION_CONTRACT_INVALID",
                f"prediction {prediction['scenarioId']} is missing required fields: {', '.join(missing)}; return every required field",
            )
        if family == "ask":
            citations = prediction.get("citations")
            if not isinstance(prediction.get("answer"), str) or not isinstance(citations, list) or not citations or not all(isinstance(item, str) and item for item in citations):
                raise RunnerError("PREDICTION_CONTRACT_INVALID", "ask prediction requires answer and non-empty string citations")
            supplied = _supplied_segment_ids(request, prediction["scenarioId"])
            invalid = sorted(set(citations) - supplied) if supplied else []
            if invalid:
                raise RunnerError(
                    "PREDICTION_CONTRACT_INVALID",
                    f"ask citations must be exact supplied segment IDs; invalid citations: {invalid}",
                )
        if family in {"organize", "validator"} and not isinstance(prediction.get("mutations"), list):
            raise RunnerError("PREDICTION_CONTRACT_INVALID", "mutation prediction requires mutations array")
        if family == "organize" and not prediction["mutations"]:
            raise RunnerError(
                "PREDICTION_CONTRACT_INVALID",
                "organize prediction requires at least one evidence-backed mutation",
            )
        if family == "organize":
            for index, mutation in enumerate(prediction["mutations"]):
                _validate_typed_organize_mutation(mutation, index)
        if family == "validator":
            correction = prediction.get("correction")
            if not isinstance(correction, dict) or not isinstance(correction.get("errorCode"), str) or not isinstance(correction.get("correctedOperation"), dict) or correction.get("retry") is not False:
                raise RunnerError("PREDICTION_CONTRACT_INVALID", "validator correction must be typed and non-retrying")
        if family in {"partition", "extract", "none", "nested"}:
            expected_mode = "partition" if family in {"partition", "nested"} else family
            if prediction.get("mode") != expected_mode:
                raise RunnerError(
                    "PREDICTION_CONTRACT_INVALID",
                    f"{family} task requires mode exactly '{expected_mode}'",
                )
            if not isinstance(prediction.get("children"), list):
                raise RunnerError(
                    "PREDICTION_CONTRACT_INVALID",
                    f"{family} task requires children as an array",
                )
            if not isinstance(prediction.get("uncovered_ranges"), list):
                raise RunnerError(
                    "PREDICTION_CONTRACT_INVALID",
                    f"{family} task requires uncovered_ranges as an array",
                )
            if family == "none":
                if prediction["children"]:
                    raise RunnerError(
                        "PREDICTION_CONTRACT_INVALID",
                        "none task requires children exactly []",
                    )
                if prediction["uncovered_ranges"]:
                    raise RunnerError(
                        "PREDICTION_CONTRACT_INVALID",
                        "none task requires uncovered_ranges exactly []",
                    )
                if "uncoveredSpans" in required and prediction.get("uncoveredSpans") != []:
                    raise RunnerError(
                        "PREDICTION_CONTRACT_INVALID",
                        "none task requires uncoveredSpans exactly []",
                    )
                if "semanticRationale" in required:
                    rationale = prediction.get("semanticRationale")
                    if not isinstance(rationale, dict):
                        raise RunnerError(
                            "PREDICTION_CONTRACT_INVALID",
                            "none task requires semanticRationale with topicSupport=insufficient and evidenceSegmentIds",
                        )
                    if rationale.get("topicSupport") != "insufficient" or not isinstance(rationale.get("evidenceSegmentIds"), list) or not rationale["evidenceSegmentIds"]:
                        raise RunnerError(
                            "PREDICTION_CONTRACT_INVALID",
                            "none task semanticRationale requires topicSupport=insufficient and non-empty evidenceSegmentIds",
                        )
            if family == "nested" and not isinstance(prediction.get("second_pass"), dict):
                raise RunnerError(
                    "PREDICTION_CONTRACT_INVALID",
                    "nested prediction requires second_pass with explicit parentChildId",
                )
            if family == "nested":
                second_pass = prediction["second_pass"]
                if second_pass.get("mode") != "partition" or not isinstance(second_pass.get("children"), list) or not isinstance(second_pass.get("uncovered_ranges"), list):
                    raise RunnerError(
                        "PREDICTION_CONTRACT_INVALID",
                        "nested second_pass requires mode=partition, children, and uncovered_ranges",
                    )
                if not isinstance(second_pass.get("parent_child_index"), int) and not isinstance(second_pass.get("parentChildId"), str):
                    raise RunnerError(
                        "PREDICTION_CONTRACT_INVALID",
                        "nested second_pass requires parent_child_index or parentChildId",
                    )
            if family == "nested" and "secondPass" in required:
                second_pass = prediction.get("secondPass")
                alias = prediction.get("second_pass")
                if not isinstance(second_pass, dict) or not isinstance(alias, dict):
                    raise RunnerError(
                        "PREDICTION_CONTRACT_INVALID",
                        "nested prediction requires both secondPass and second_pass objects",
                    )
                parent_id = second_pass.get("parentChildId")
                alias_parent_id = alias.get("parentChildId")
                if not isinstance(parent_id, str) or not parent_id:
                    raise RunnerError(
                        "PREDICTION_CONTRACT_INVALID",
                        "nested prediction requires secondPass.parentChildId selecting one emitted root child",
                    )
                if alias_parent_id != parent_id:
                    raise RunnerError(
                        "PREDICTION_CONTRACT_INVALID",
                        "nested prediction secondPass and second_pass must select the same parentChildId",
                    )
            _validate_semantic_structure(request, prediction)


def _result_metadata(config: Configuration, response: dict[str, Any], tool_calls: list[dict[str, Any]], model_metadata: dict[str, Any]) -> dict[str, Any]:
    metadata = {
        "predictionSource": "external_runner",
        "transport": "http",
        "protocol": config.protocol,
        "model": config.model,
        "toolCalls": tool_calls,
        "readback": None,
        "mutationReceipts": [],
        "receipt": None,
        "receiptAvailable": False,
    }
    if isinstance(model_metadata, dict):
        for field in ("toolCalls", "readback", "readbacks", "mutationReceipts", "receipt", "receipts", "receiptAvailable"):
            if field in model_metadata:
                metadata[field] = model_metadata[field]
    usage = response.get("usage")
    if usage is not None:
        metadata["usage"] = _safe_metadata(usage)
    serialized = json.dumps(metadata, ensure_ascii=False).encode("utf-8")
    if len(serialized) > MAX_METADATA_BYTES:
        raise RunnerError("METADATA_SIZE_EXCEEDED", "tool/readback/receipt metadata exceeds the byte limit")
    return metadata


def _enforce_output_limits(
    request: dict[str, Any], predictions: list[dict[str, Any]], metadata: dict[str, Any]
) -> None:
    resources = request.get("resources", {})
    if isinstance(resources, dict):
        max_prediction_bytes = int(
            resources.get("maxPredictionBytes", DEFAULT_MAX_RESPONSE_BYTES)
        )
        max_receipts = int(resources.get("maxMutationReceipts", 16))
    else:
        max_prediction_bytes = DEFAULT_MAX_RESPONSE_BYTES
        max_receipts = 16
    if len(json.dumps(predictions, ensure_ascii=False).encode("utf-8")) > max_prediction_bytes:
        raise RunnerError("PREDICTION_SIZE_EXCEEDED", "predictions exceed the configured byte limit")
    receipts = metadata.get("mutationReceipts", [])
    if not isinstance(receipts, list):
        raise RunnerError("METADATA_INVALID", "mutationReceipts metadata must be an array")
    if len(receipts) > max_receipts:
        raise RunnerError(
            "MUTATION_RECEIPTS_EXCEEDED",
            "mutation receipt count exceeds the configured limit",
        )


def run_request(request: Any, environment: Mapping[str, str] | None = None) -> dict[str, Any]:
    public = validate_public_request(request)
    if len(public["records"]) > 1:
        # Keep each model response inside the fixed output budget.  The matrix
        # still receives one complete prediction set, while the runner makes
        # one independently auditable request per scenario and never asks the
        # model to squeeze dozens of JSON objects into a 1K-token response.
        predictions: list[dict[str, Any]] = []
        metadata: list[dict[str, Any]] = []
        for record in public["records"]:
            single = copy.deepcopy(public)
            single["records"] = [record]
            result = run_request(single, environment)
            predictions.extend(result["predictions"])
            metadata.append(result.get("metadata", {}))
        tool_calls = [
            call
            for item in metadata
            for call in (item.get("toolCalls") if isinstance(item.get("toolCalls"), list) else [])
        ]
        readbacks = [
            item.get("readback")
            for item in metadata
            if item.get("readback") is not None
        ]
        mutation_receipts = [
            receipt
            for item in metadata
            for receipt in (
                item.get("mutationReceipts")
                if isinstance(item.get("mutationReceipts"), list)
                else []
            )
        ]
        combined_metadata = {
            "predictionSource": "external_runner",
            "transport": "http",
            "batchedRequests": len(metadata),
            "requests": metadata,
            "toolCalls": tool_calls,
            "readbacks": readbacks,
            "mutationReceipts": mutation_receipts,
            "receiptAvailable": any(item.get("receiptAvailable") is True for item in metadata),
        }
        if len(json.dumps(combined_metadata, ensure_ascii=False).encode("utf-8")) > MAX_METADATA_BYTES:
            raise RunnerError("METADATA_SIZE_EXCEEDED", "batched metadata exceeds the byte limit")
        _enforce_output_limits(public, predictions, combined_metadata)
        return {
            "schemaVersion": 1,
            "status": "completed",
            "predictions": predictions,
            "metadata": combined_metadata,
        }
    config = _protocol_config(
        public["protocol"], os.environ if environment is None else environment
    )
    resources = public.get("resources", {}) if isinstance(public.get("resources"), dict) else {}
    timeout = max(0.1, float(resources.get("timeoutMs", int(DEFAULT_TIMEOUT * 1000))) / 1000)
    max_bytes = int(resources.get("maxPredictionBytes", DEFAULT_MAX_RESPONSE_BYTES))
    repair_note = ""
    # Three total HTTP attempts is the hard repair bound.  The runner never
    # manufactures a missing field; every retry sends a new public prompt and
    # accepts only a complete model response.
    for attempt in range(3):
        request_for_model = public
        if repair_note:
            request_for_model = copy.deepcopy(public)
            prompt = dict(request_for_model.get("prompt", {}))
            prompt["task"] = f"{prompt.get('task', '')}\n\nREPAIR REQUIRED: {repair_note}"
            request_for_model["prompt"] = prompt
        payload = build_payload(request_for_model, config)
        try:
            response = _http_json(config, payload, timeout, max_bytes)
            parsed, tool_calls = _extract_result(response, config.protocol)
            predictions, model_metadata = _prediction_list(parsed)
            _validate_predictions(public, predictions)
            break
        except RunnerError as error:
            if attempt < 2 and error.code in {"MODEL_OUTPUT_INVALID", "MODEL_INCOMPLETE", "PREDICTION_CONTRACT_INVALID"}:
                repair_note = error.reason
                continue
            raise
    result_metadata = _result_metadata(config, response, tool_calls, model_metadata)
    _enforce_output_limits(public, predictions, result_metadata)
    return {
        "schemaVersion": 1,
        "status": "completed",
        "predictions": predictions,
        "metadata": result_metadata,
    }


def _error_payload(error: RunnerError) -> dict[str, Any]:
    return {
        "schemaVersion": 1,
        "status": error.status,
        "predictions": [],
        "errorCode": error.code,
        "reason": error.reason,
        "metadata": {
            "predictionSource": "external_runner",
            "transport": "http",
            "toolCalls": [],
            "readback": None,
            "mutationReceipts": [],
            "receipt": None,
            "receiptAvailable": False,
        },
    }


def main() -> int:
    try:
        raw_bytes = sys.stdin.buffer.read(DEFAULT_MAX_INPUT_BYTES + 1)
        if len(raw_bytes) > DEFAULT_MAX_INPUT_BYTES:
            raise RunnerError("INPUT_SIZE_EXCEEDED", "stdin exceeds the default byte limit", status="not_assessed")
        raw = raw_bytes.decode("utf-8")
        if not raw.strip():
            raise RunnerError("INVALID_REQUEST", "stdin is empty", status="not_assessed")
        request = json.loads(raw)
        result = run_request(request)
    except RunnerError as error:
        result = _error_payload(error)
    except json.JSONDecodeError:
        result = _error_payload(RunnerError("INVALID_REQUEST_JSON", "stdin is not valid JSON", status="not_assessed"))
    except UnicodeDecodeError:
        result = _error_payload(RunnerError("INVALID_REQUEST_ENCODING", "stdin must be UTF-8", status="not_assessed"))
    except Exception:
        # Keep stdout machine-readable without exposing endpoint, credentials,
        # or a traceback.  Unexpected implementation failures are assessable
        # as failed, never as a completed prediction.
        result = _error_payload(RunnerError("RUNNER_INTERNAL_ERROR", "runner failed before producing a prediction"))
    sys.stdout.write(json.dumps(result, ensure_ascii=False, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
