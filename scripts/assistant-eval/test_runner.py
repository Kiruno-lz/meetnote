#!/usr/bin/env python3

import json
import os
import subprocess
import sys
import threading
import unittest
from copy import deepcopy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent))

from runner import RunnerError, _protocol_config, _public_prompt, build_payload, run_request


ROOT = Path(__file__).resolve().parent


def request_for(protocol="openaiChatCompletions", family="ask"):
    required = {
        "ask": ["scenarioId", "answer", "citations"],
        "organize": ["scenarioId", "mutations"],
        "validator": ["scenarioId", "mutations", "correction"],
        "partition": ["scenarioId", "mode", "children", "uncovered_ranges"],
        "extract": ["scenarioId", "mode", "children", "uncovered_ranges"],
        "none": ["scenarioId", "mode", "children", "uncovered_ranges"],
        "nested": ["scenarioId", "mode", "children", "uncovered_ranges", "second_pass"],
    }[family]
    return {
        "schemaVersion": 1,
        "protocol": protocol,
        "caseId": "case-1",
        "taskFamily": family,
        "prompt": {"system": "Use supplied evidence only", "task": "Return typed JSON"},
        "budget": {"maxOutputTokens": 128},
        "resources": {"timeoutMs": 3000, "maxPredictionBytes": 65536},
        "records": [{
            "scenarioId": "scenario-1",
            "record": {
                "public": True,
                "state": {"results": {"transcript": [{"id": "seg-1", "text": "supplied evidence"}]}},
            },
        }],
        "outputContract": {"required": required},
    }


def prediction_for(family):
    prediction = {"scenarioId": "scenario-1"}
    if family == "ask":
        prediction.update(answer="supported answer", citations=["seg-1"])
    elif family == "organize":
        prediction["mutations"] = [{"operation": "update_document", "patch": {"title": "fixture"}}]
    elif family == "validator":
        prediction.update(
            mutations=[],
            correction={
                "errorCode": "RANGE_OVERLAP",
                "correctedOperation": {"operation": "read_document"},
                "retry": False,
            },
        )
    else:
        mode = "none" if family == "none" else family if family == "extract" else "partition"
        children = []
        if family in {"partition", "nested"}:
            children = [{
                "start_segment_id": "seg-1",
                "end_segment_id": "seg-1",
                "evidence_segment_ids": ["seg-1"],
            }]
        uncovered_ranges = [{
            "start_segment_id": "seg-1",
            "end_segment_id": "seg-1",
        }] if family == "extract" else []
        prediction.update(mode=mode, children=children, uncovered_ranges=uncovered_ranges)
        if family == "nested":
            prediction["second_pass"] = {
                "parent_child_index": 0,
                "mode": "partition",
                "children": [{
                    "start_segment_id": "seg-1",
                    "end_segment_id": "seg-1",
                    "evidence_segment_ids": ["seg-1"],
                }],
                "uncovered_ranges": [],
            }
    return prediction


class FixtureHandler(BaseHTTPRequestHandler):
    protocol_name = "openaiChatCompletions"
    family = "ask"
    response_mode = "valid"
    include_metadata = False
    include_tool_call = False
    requests = []

    def do_POST(self):  # noqa: N802 - BaseHTTPRequestHandler API
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        self.__class__.requests.append((self.path, dict(self.headers), json.loads(body)))
        if self.__class__.response_mode == "http_error":
            payload = {"error": {"message": "credential=should-not-leak"}}
            self.send_response(500)
        elif self.__class__.response_mode == "model_error":
            payload = {"error": {"message": "credential=should-not-leak"}}
            self.send_response(200)
        else:
            if self.__class__.response_mode == "invalid_model":
                text = "not JSON"
            else:
                envelope = {
                    "predictions": [prediction_for(self.__class__.family)],
                    "metadata": {
                        "readback": {"status": "confirmed", "api_key": "fixture-secret"},
                        "mutationReceipts": [],
                        "receipt": None,
                        "receiptAvailable": False,
                    },
                }
                if self.__class__.response_mode == "oracle":
                    envelope["metadata"]["gold"] = {"copied": True}
                if self.__class__.include_metadata:
                    envelope["metadata"] = {
                        "toolCalls": [{"id": "model-call-1", "name": "readback", "arguments": {}}],
                        "readback": {"status": "confirmed", "revision": 4},
                        "mutationReceipts": [{"receiptId": "receipt-1"}],
                        "receipt": {"source": "model"},
                        "receiptAvailable": True,
                    }
                text = json.dumps(envelope)
            if self.__class__.protocol_name == "openaiChatCompletions":
                message = {"content": text}
                if self.__class__.include_tool_call:
                    message["tool_calls"] = [
                        {
                            "id": "http-call-1",
                            "type": "function",
                            "function": {"name": "readback", "arguments": "{}"},
                        }
                    ]
                payload = {"model": "fixture-model", "choices": [{"message": message, "finish_reason": "stop"}]}
            elif self.__class__.protocol_name == "anthropicMessages":
                payload = {"model": "fixture-model", "content": [{"type": "text", "text": text}], "stop_reason": "end_turn"}
            else:
                payload = {
                    "model": "fixture-model",
                    "status": "completed",
                    "output": [{"type": "message", "content": [{"type": "output_text", "text": text}]}],
                }
            self.send_response(200)
        encoded = json.dumps(payload).encode()
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *_args):
        return


class LocalModel:
    def __init__(self, protocol, family="ask", response_mode="valid", include_metadata=False, include_tool_call=False):
        handler = type("Handler", (FixtureHandler,), {})
        handler.protocol_name = protocol
        handler.family = family
        handler.response_mode = response_mode
        handler.include_metadata = include_metadata
        handler.include_tool_call = include_tool_call
        handler.requests = []
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.handler = handler
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    @property
    def base_url(self):
        return f"http://127.0.0.1:{self.server.server_port}/v1"

    @property
    def requests(self):
        return self.handler.requests

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.close()


def environment(protocol, endpoint, model="fixture-model", key="fixture-key"):
    values = {
        "MEETNOTE_ASSISTANT_EVAL_MODEL": model,
        "MEETNOTE_ASSISTANT_EVAL_API_KEY": key,
    }
    if protocol == "openaiChatCompletions":
        values["MEETNOTE_ASSISTANT_EVAL_OPENAI_CHAT_COMPLETIONS_ENDPOINT"] = endpoint
    elif protocol == "anthropicMessages":
        values["MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_MESSAGES_ENDPOINT"] = endpoint
    else:
        values["MEETNOTE_ASSISTANT_EVAL_OPENAI_RESPONSES_ENDPOINT"] = endpoint
    return values


class RunnerTests(unittest.TestCase):
    def test_three_protocols_build_payload_and_parse_structured_output(self):
        for protocol in ("openaiChatCompletions", "anthropicMessages", "openaiResponses"):
            with self.subTest(protocol=protocol), LocalModel(protocol) as model:
                result = run_request(request_for(protocol), environment(protocol, model.base_url))
                self.assertEqual(result["status"], "completed")
                self.assertEqual(result["predictions"][0]["scenarioId"], "scenario-1")
                self.assertEqual(result["metadata"]["predictionSource"], "external_runner")
                path, headers, payload = model.requests[0]
                self.assertTrue(path.endswith({
                    "openaiChatCompletions": "/v1/chat/completions",
                    "anthropicMessages": "/v1/messages",
                    "openaiResponses": "/v1/responses",
                }[protocol]))
                self.assertEqual(payload["model"], "fixture-model")
                if protocol == "anthropicMessages":
                    self.assertIn("x-api-key", {key.lower(): value for key, value in headers.items()})
                else:
                    self.assertIn("authorization", {key.lower(): value for key, value in headers.items()})

    def test_structured_operation_families_and_metadata_are_preserved(self):
        for family in ("ask", "organize", "validator", "partition", "extract", "none", "nested"):
            with self.subTest(family=family), LocalModel("openaiResponses", family) as model:
                result = run_request(request_for("openaiResponses", family), environment("openaiResponses", model.base_url))
                self.assertEqual(result["predictions"][0]["scenarioId"], "scenario-1")
                self.assertEqual(result["metadata"]["readback"]["status"], "confirmed")
                self.assertEqual(result["metadata"]["readback"]["api_key"], "[redacted]")
                self.assertEqual(result["metadata"]["mutationReceipts"], [])
                self.assertFalse(result["metadata"]["receiptAvailable"])

    def test_validator_and_organize_prompts_state_repairable_typed_contracts(self):
        validator_prompt = _public_prompt(request_for("openaiResponses", "validator"))
        self.assertIn("non-empty errorCode", validator_prompt)
        self.assertIn("retry exactly false", validator_prompt)
        organize_prompt = _public_prompt(request_for("openaiResponses", "organize"))
        self.assertIn("at least one typed mutation", organize_prompt)
        self.assertIn("non-empty patch", organize_prompt)
        self.assertNotIn("segment ranges", organize_prompt.split("For organize tasks", 1)[1])

    def test_validator_contract_repair_accepts_only_non_retrying_typed_correction(self):
        request = request_for("openaiChatCompletions", "validator")
        invalid = {
            "scenarioId": "scenario-1",
            "mutations": [],
            "correction": {"errorCode": "RANGE_OVERLAP", "correctedOperation": {}, "retry": True},
        }
        valid = {
            "scenarioId": "scenario-1",
            "mutations": [],
            "correction": {"errorCode": "RANGE_OVERLAP", "correctedOperation": {"operation": "read_document"}, "retry": False},
        }
        def response(prediction):
            return {"choices": [{"message": {"content": json.dumps({"predictions": [prediction]})}, "finish_reason": "stop"}]}
        with patch("runner._http_json", side_effect=[response(invalid), response(valid)]) as http:
            result = run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))
        self.assertEqual(result["status"], "completed")
        self.assertEqual(http.call_count, 2)
        self.assertIn("REPAIR REQUIRED", http.call_args_list[1].args[1]["messages"][1]["content"])

    def test_multi_record_batching_is_per_record_and_global_output_limit_is_enforced(self):
        request = request_for("openaiChatCompletions", "ask")
        second = deepcopy(request["records"][0])
        second["scenarioId"] = "scenario-2"
        request["records"].append(second)

        def response(scenario_id):
            return {"choices": [{"message": {"content": json.dumps({"predictions": [{
                "scenarioId": scenario_id,
                "answer": "supported answer",
                "citations": ["seg-1"],
            }]})}, "finish_reason": "stop"}]}

        with patch("runner._http_json", side_effect=[response("scenario-1"), response("scenario-2")]) as http:
            result = run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))
        self.assertEqual([item["scenarioId"] for item in result["predictions"]], ["scenario-1", "scenario-2"])
        self.assertEqual(result["metadata"]["batchedRequests"], 2)
        self.assertEqual(http.call_count, 2)

        request["resources"]["maxPredictionBytes"] = 64
        with patch("runner._http_json", side_effect=[response("scenario-1"), response("scenario-2")]):
            with self.assertRaisesRegex(RunnerError, "predictions exceed"):
                run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))

    def test_contract_repair_retries_at_most_three_times_with_explicit_hints(self):
        request = request_for("openaiChatCompletions", "none")
        request["outputContract"]["required"].extend(["uncoveredSpans", "semanticRationale"])
        invalid_missing = {
            "scenarioId": "scenario-1",
            "mode": "none",
            "children": [],
            "uncovered_ranges": [],
            "uncoveredSpans": [],
        }
        invalid_children = {
            **invalid_missing,
            "children": [{"childId": "must-not-exist"}],
            "semanticRationale": {"topicSupport": "insufficient", "evidenceSegmentIds": ["seg-1"]},
        }
        valid = {
            **invalid_missing,
            "semanticRationale": {"topicSupport": "insufficient", "evidenceSegmentIds": ["seg-1"]},
        }

        def response(prediction):
            return {
                "choices": [{
                    "message": {"content": json.dumps({"predictions": [prediction]})},
                    "finish_reason": "stop",
                }]
            }

        with patch(
            "runner._http_json",
            side_effect=[response(invalid_missing), response(invalid_children), response(valid)],
        ) as http:
            result = run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))
        self.assertEqual(result["status"], "completed")
        self.assertEqual(http.call_count, 3)
        repair_payload = http.call_args_list[1].args[1]
        self.assertIn("REPAIR REQUIRED", repair_payload["messages"][1]["content"])
        self.assertIn("semanticRationale", repair_payload["messages"][1]["content"])
        final_payload = http.call_args_list[2].args[1]
        self.assertIn("children exactly []", final_payload["messages"][1]["content"])

    def test_contract_repair_never_exceeds_three_attempts(self):
        request = request_for("openaiChatCompletions", "none")
        request["outputContract"]["required"].extend(["uncoveredSpans", "semanticRationale"])
        invalid = {
            "scenarioId": "scenario-1",
            "mode": "none",
            "children": [],
            "uncovered_ranges": [],
            "uncoveredSpans": [],
        }
        response = {
            "choices": [{
                "message": {"content": json.dumps({"predictions": [invalid]})},
                "finish_reason": "stop",
            }]
        }
        with patch("runner._http_json", return_value=response) as http:
            with self.assertRaisesRegex(RunnerError, "semanticRationale"):
                run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))
        self.assertEqual(http.call_count, 3)

    def test_organize_requires_non_empty_mutations(self):
        with LocalModel("openaiChatCompletions", "organize") as model:
            # Replace the fixture response with an empty mutation proposal.
            original = model.handler.response_mode
            model.handler.response_mode = "valid"
            with self.assertRaisesRegex(RunnerError, "at least one evidence-backed mutation"):
                request = request_for("openaiChatCompletions", "organize")
                # The handler uses prediction_for; use a patched HTTP response so
                # the test exercises the contract rather than a server branch.
                empty = {"scenarioId": "scenario-1", "mutations": []}
                response = {"choices": [{"message": {"content": json.dumps({"predictions": [empty]})}, "finish_reason": "stop"}]}
                with patch("runner._http_json", return_value=response):
                    run_request(request, environment("openaiChatCompletions", model.base_url))
            model.handler.response_mode = original

    def test_ask_citations_must_reference_supplied_segment_ids(self):
        request = request_for("openaiChatCompletions", "ask")
        response = {"choices": [{"message": {"content": json.dumps({"predictions": [{"scenarioId": "scenario-1", "answer": "answer", "citations": ["not-supplied"]}]})}, "finish_reason": "stop"}]}
        with patch("runner._http_json", return_value=response):
            with self.assertRaisesRegex(RunnerError, "exact supplied segment IDs"):
                run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))

    def test_organize_mutations_must_be_typed(self):
        request = request_for("openaiChatCompletions", "organize")
        response = {"choices": [{"message": {"content": json.dumps({"predictions": [{"scenarioId": "scenario-1", "mutations": [{"unsupported": True}]}]})}, "finish_reason": "stop"}]}
        with patch("runner._http_json", return_value=response):
            with self.assertRaisesRegex(RunnerError, "typed op or non-empty patch"):
                run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))

    def test_update_document_forms_allow_fields_and_reject_range_fields(self):
        request = request_for("openaiChatCompletions", "organize")
        valid = {"scenarioId": "scenario-1", "mutations": [{
            "op": "update_document",
            "fields": {"document_status": "ready"},
            "evidenceSegmentIds": ["seg-1"],
            "description": "Evidence explanation",
        }]}
        response = {"choices": [{"message": {"content": json.dumps({"predictions": [valid]})}, "finish_reason": "stop"}]}
        with patch("runner._http_json", return_value=response):
            self.assertEqual(run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))["status"], "completed")
        invalid_aliases = {"scenarioId": "scenario-1", "mutations": [{
            "op": "update_document",
            "fields": {"document_status": "ready", "documentStatus": "ready"},
        }]}
        response["choices"][0]["message"]["content"] = json.dumps({"predictions": [invalid_aliases]})
        with patch("runner._http_json", return_value=response):
            with self.assertRaisesRegex(RunnerError, "both document_status and documentStatus"):
                run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))
        invalid = {"scenarioId": "scenario-1", "mutations": [{
            "op": "update_document",
            "fields": {"title": "bad"},
            "start_segment_id": "seg-1",
        }]}
        response["choices"][0]["message"]["content"] = json.dumps({"predictions": [invalid]})
        with patch("runner._http_json", return_value=response):
            with self.assertRaisesRegex(RunnerError, "contains range fields"):
                run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))

    def test_responses_deepseek_non_thinking_hints_are_provider_compatible(self):
        request = request_for("openaiResponses", "validator")
        config = _protocol_config("openaiResponses", {
            "MEETNOTE_ASSISTANT_EVAL_MODEL": "deepseek-v4-flash",
            "MEETNOTE_ASSISTANT_EVAL_API_KEY": "fixture-key",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_RESPONSES_ENDPOINT": "https://example.test/v1",
        })
        with patch.dict("runner.os.environ", {"MEETNOTE_ASSISTANT_EVAL_DISABLE_THINKING": "1"}, clear=False):
            payload = build_payload(request, config)
        self.assertEqual(payload["reasoning"], {"effort": "none"})
        self.assertEqual(payload["thinking"], {"type": "disabled"})
        self.assertEqual(payload["reasoning_effort"], "none")

    def test_responses_incomplete_is_explicit_and_not_parsed_as_prediction(self):
        request = request_for("openaiResponses", "validator")
        response = {
            "status": "incomplete",
            "incomplete_details": {"reason": "max_output_tokens"},
            "output": [{"type": "reasoning", "summary": []}],
        }
        with patch("runner._http_json", return_value=response):
            with self.assertRaisesRegex(RunnerError, "MODEL_INCOMPLETE|incomplete status"):
                run_request(request, environment("openaiResponses", "https://example.test/v1"))

    def test_nested_second_pass_requires_typed_fields(self):
        request = request_for("openaiChatCompletions", "nested")
        response = {"choices": [{"message": {"content": json.dumps({"predictions": [{"scenarioId": "scenario-1", "mode": "partition", "children": [], "uncovered_ranges": [], "second_pass": {"children": [], "uncovered_ranges": []}}]})}, "finish_reason": "stop"}]}
        with patch("runner._http_json", return_value=response):
            with self.assertRaisesRegex(RunnerError, "mode=partition"):
                run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))

    def test_range_contract_rejects_omitted_segments_before_matrix(self):
        request = request_for("openaiChatCompletions", "partition")
        response = {"choices": [{"message": {"content": json.dumps({"predictions": [{"scenarioId": "scenario-1", "mode": "partition", "children": [], "uncovered_ranges": []}]})}, "finish_reason": "stop"}]}
        with patch("runner._http_json", return_value=response):
            with self.assertRaisesRegex(RunnerError, "does not cover every segment"):
                run_request(request, environment("openaiChatCompletions", "https://example.test/v1"))

    def test_tool_readback_and_receipt_metadata_are_preserved(self):
        with LocalModel("openaiChatCompletions", "organize", include_metadata=True) as model:
            result = run_request(
                request_for("openaiChatCompletions", "organize"),
                environment("openaiChatCompletions", model.base_url),
            )
        self.assertEqual(result["metadata"]["toolCalls"][0]["id"], "model-call-1")
        self.assertEqual(result["metadata"]["readback"]["revision"], 4)
        self.assertEqual(result["metadata"]["mutationReceipts"][0]["receiptId"], "receipt-1")
        self.assertTrue(result["metadata"]["receiptAvailable"])

    def test_http_tool_call_is_retained_when_final_json_is_present(self):
        with LocalModel("openaiChatCompletions", include_tool_call=True) as model:
            result = run_request(
                request_for("openaiChatCompletions"),
                environment("openaiChatCompletions", model.base_url),
            )
        self.assertEqual(result["metadata"]["toolCalls"][0]["id"], "http-call-1")

    def test_request_rejects_evaluator_only_fields_without_network_call(self):
        for field in ("gold", "negativeCases", "semanticRationale", "rationale"):
            with self.subTest(field=field):
                request = request_for()
                request["records"][0][field] = {"forbidden": True}
                with self.assertRaises(RunnerError) as context:
                    run_request(request, {})
                self.assertEqual(context.exception.status, "not_assessed")
                self.assertEqual(context.exception.code, "EVALUATOR_DATA_IN_REQUEST")

    def test_missing_configuration_is_not_assessed(self):
        with self.assertRaises(RunnerError) as context:
            run_request(request_for(), {})
        self.assertEqual(context.exception.status, "not_assessed")
        self.assertEqual(context.exception.code, "MISSING_CONFIGURATION")

    def test_http_error_and_invalid_model_output_are_failed(self):
        with LocalModel("openaiChatCompletions", response_mode="http_error") as model:
            with self.assertRaises(RunnerError) as context:
                run_request(request_for(), environment("openaiChatCompletions", model.base_url))
            self.assertEqual(context.exception.status, "failed")
            self.assertEqual(context.exception.code, "HTTP_ERROR")
            self.assertNotIn("should-not-leak", context.exception.reason)
        with LocalModel("openaiChatCompletions", response_mode="invalid_model") as model:
            with self.assertRaisesRegex(RunnerError, "one JSON value"):
                run_request(request_for(), environment("openaiChatCompletions", model.base_url))
        with LocalModel("openaiChatCompletions", response_mode="model_error") as model:
            with self.assertRaises(RunnerError) as context:
                run_request(request_for(), environment("openaiChatCompletions", model.base_url))
            self.assertEqual(context.exception.code, "MODEL_ERROR")
            self.assertNotIn("should-not-leak", context.exception.reason)

    def test_input_limit_is_explicitly_not_assessed(self):
        request = request_for()
        request["resources"]["maxInputBytes"] = 1
        with self.assertRaises(RunnerError) as context:
            run_request(request, {})
        self.assertEqual(context.exception.status, "not_assessed")
        self.assertEqual(context.exception.code, "INPUT_SIZE_EXCEEDED")

    def test_model_oracle_fields_are_rejected(self):
        with LocalModel("openaiChatCompletions", response_mode="oracle") as model:
            with self.assertRaises(RunnerError) as context:
                run_request(
                    request_for(),
                    environment("openaiChatCompletions", model.base_url),
                )
        self.assertEqual(context.exception.code, "ORACLE_OUTPUT_REJECTED")

    def test_cli_emits_one_machine_readable_not_assessed_result(self):
        completed = subprocess.run(
            [sys.executable, str(ROOT / "runner.py")],
            input=json.dumps(request_for()),
            text=True,
            capture_output=True,
            env={key: value for key, value in os.environ.items() if not key.startswith("MEETNOTE_ASSISTANT_EVAL_")},
            check=False,
        )
        self.assertEqual(completed.returncode, 0)
        self.assertEqual(completed.stderr, "")
        result = json.loads(completed.stdout)
        self.assertEqual(result["status"], "not_assessed")
        self.assertEqual(result["predictions"], [])
        self.assertEqual(result["errorCode"], "MISSING_CONFIGURATION")

    def test_process_cli_round_trips_one_json_line_to_http(self):
        with LocalModel("openaiChatCompletions") as model:
            env = os.environ.copy()
            env.update(environment("openaiChatCompletions", model.base_url))
            completed = subprocess.run(
                [sys.executable, str(ROOT / "runner.py")],
                input=json.dumps(request_for()),
                text=True,
                capture_output=True,
                env=env,
                check=False,
            )
        self.assertEqual(completed.returncode, 0)
        self.assertEqual(completed.stderr, "")
        self.assertEqual(len(completed.stdout.strip().splitlines()), 1)
        self.assertEqual(json.loads(completed.stdout)["status"], "completed")


if __name__ == "__main__":
    unittest.main()
