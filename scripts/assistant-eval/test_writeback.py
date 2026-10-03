import json
import os
import tempfile
import unittest
from pathlib import Path

from writeback import (
    WritebackConfig,
    WritebackError,
    _public_record,
    create_temporary_document_package,
    markdown_sha256,
    _mutation_patch,
    _reject_evaluator_prediction_fields,
    writeback_prediction,
)


ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "crates/application/tests/fixtures/assistant/synthetic-corpus.jsonl"
MEETNOTE = ROOT / "target/debug/meetnote"


class WritebackContractTests(unittest.TestCase):
    def test_public_record_does_not_require_evaluator_gold(self):
        record = {"record": {"state": {}, "markdown": "# public"}, "gold": {"secret": True}}
        self.assertEqual(_public_record(record)["markdown"], "# public")

    def test_markdown_hash_is_stable(self):
        self.assertEqual(markdown_sha256("text"), markdown_sha256("text"))
        self.assertNotEqual(markdown_sha256("text"), markdown_sha256("text\n"))

    def test_missing_public_record_is_structured(self):
        with self.assertRaises(WritebackError) as raised:
            _public_record({})
        self.assertEqual(raised.exception.code, "PUBLIC_RECORD_INVALID")

    def test_prediction_with_gold_is_rejected_before_workspace_write(self):
        with self.assertRaises(WritebackError) as raised:
            writeback_prediction(
                {"scenarioId": "scenario", "gold": {"children": []}},
                {"record": {"state": {}, "markdown": "# public"}},
                WritebackConfig(cli=["unreachable"], package_root=Path(tempfile.gettempdir())),
                family="partition",
                run_key="gold-rejection",
            )
        self.assertEqual(raised.exception.code, "EVALUATOR_DATA_IN_PREDICTION")

    def test_model_rationale_is_allowed_but_evaluator_labels_are_rejected(self):
        for value in (
            {"rationale": "model-generated explanation"},
            {"semanticRationale": {"evidenceSegmentIds": ["seg-1"]}},
        ):
            _reject_evaluator_prediction_fields(value)
        for value in (
            {"gold": {"children": []}},
            {"negativeCase": {"expected_error_code": "X"}},
            {"annotation": {"label": "private"}},
            {"queries": [{"answer": "private"}]},
            {"rationale": {"gold": {"children": []}}},
        ):
            with self.subTest(value=value):
                with self.assertRaises(WritebackError) as raised:
                    _reject_evaluator_prediction_fields(value)
                self.assertEqual(raised.exception.code, "EVALUATOR_DATA_IN_PREDICTION")

    def test_package_root_cannot_escape_eval_data_or_system_temp(self):
        for package_root in (Path("/etc"), ROOT / "target/assistant-eval-writeback"):
            with self.subTest(package_root=package_root):
                with self.assertRaises(WritebackError) as raised:
                    create_temporary_document_package({}, package_root, "unsafe-path")
                self.assertEqual(raised.exception.code, "WRITEBACK_PATH_INVALID")

    def test_package_root_under_eval_data_passes_path_guard(self):
        with self.assertRaises(WritebackError) as raised:
            create_temporary_document_package({}, ROOT / "eval-data/assistant/writeback", "eval-data-path")
        self.assertEqual(raised.exception.code, "PUBLIC_RECORD_INVALID")

    def test_typed_organize_operations_use_only_fixed_patch_mappings(self):
        self.assertEqual(
            _mutation_patch(
                {
                    "op": "set_title",
                    "value": "New title",
                    "evidence_segment_ids": ["seg-1"],
                }
            ),
            (None, {"title": "New title"}),
        )
        self.assertEqual(
            _mutation_patch({"op": "set_tags", "value": ["one", "two"]}),
            (None, {"tags": ["one", "two"]}),
        )
        self.assertEqual(
            _mutation_patch({"op": "set_summary", "value": None}),
            (None, {"summary": None}),
        )
        self.assertEqual(
            _mutation_patch({"op": "set_markdown", "value": "# body"}),
            (None, {"markdown": "# body"}),
        )
        self.assertEqual(
            _mutation_patch({"op": "set_document_status", "value": "ready"}),
            (None, {"documentStatus": "ready"}),
        )
        self.assertEqual(
            _mutation_patch({
                "op": "update_document",
                "fields": {"document_status": "ready"},
                "evidenceSegmentIds": ["seg-1"],
                "description": "Evidence explanation",
            }),
            (None, {"documentStatus": "ready"}),
        )
        for mutation, message in [
            ({"op": "set_title", "value": ""}, "non-empty"),
            ({"op": "set_tags", "value": {"arbitrary": True}}, "tags"),
            ({"op": "set_summary", "value": 1}, "string or null"),
            ({"op": "set_title", "value": "ok", "patch": {"markdown": "bad"}}, "unsupported fields"),
            ({"op": "set_title", "value": "ok", "evidence_segment_ids": []}, "non-empty string array"),
            ({"op": "set_title", "value": "ok", "evidence_segment_ids": ["seg-1", 2]}, "non-empty string array"),
            ({"op": "set_title", "value": "ok", "evidenceSegmentIds": ["seg-1", 2]}, "non-empty string array"),
            ({"op": "update_document", "fields": {"title": "bad"}, "start_segment_id": "seg-1"}, "range fields"),
        ]:
            with self.subTest(mutation=mutation):
                with self.assertRaisesRegex(WritebackError, message):
                    _mutation_patch(mutation)

    def test_partition_writeback_uses_real_receipt_and_readback(self):
        if not MEETNOTE.is_file():
            self.skipTest("target/debug/meetnote is not built")
        record = json.loads(CORPUS.read_text(encoding="utf-8").splitlines()[0])
        public_record = {key: value for key, value in record.items() if key not in {"gold", "negativeCase"}}
        segments = public_record["record"]["state"]["results"]["transcript"]
        split = len(segments) // 2
        prediction = {
            "scenarioId": public_record["scenarioId"],
            "mode": "partition",
            "children": [
                {
                    "start_segment_id": segments[0]["id"],
                    "end_segment_id": segments[split - 1]["id"],
                    "evidence_segment_ids": [segments[0]["id"]],
                    "title": "External topic one",
                    "summary": "Evidence-backed external topic one.",
                    "tags": ["external-one"],
                },
                {
                    "start_segment_id": segments[split]["id"],
                    "end_segment_id": segments[-1]["id"],
                    "evidence_segment_ids": [segments[-1]["id"]],
                    "title": "External topic two",
                    "summary": "Evidence-backed external topic two.",
                    "tags": ["external-two"],
                },
            ],
            "uncovered_ranges": [],
        }
        with tempfile.TemporaryDirectory(prefix="assistant-eval-writeback-test-") as temporary:
            evidence = writeback_prediction(
                prediction,
                public_record,
                WritebackConfig(cli=[str(MEETNOTE)], package_root=Path(temporary)),
                family="partition",
                run_key="partition-real-receipt",
            )
        receipts = evidence["mutationReceipts"]
        self.assertEqual(len(receipts), 1)
        receipt = receipts[0]
        self.assertTrue(receipt["mutationId"])
        self.assertEqual(len(receipt["affectedDocuments"]), 3)
        parent = next(item for item in receipt["affectedDocuments"] if item["documentId"] == evidence["documentId"])
        self.assertEqual(parent["previousRevision"], 0)
        self.assertEqual(parent["revision"], 1)
        self.assertEqual(len(evidence["readback"]), 3)
        for readback in evidence["readback"]:
            markdown = readback["markdown"]
            accepted_hashes = {markdown_sha256(markdown)}
            if markdown.endswith("\n"):
                accepted_hashes.add(markdown_sha256(markdown[:-1]))
            self.assertIn(readback["metadata"]["markdownSha256"], accepted_hashes)

    def test_typed_organize_writeback_uses_update_receipt_and_readback(self):
        if not MEETNOTE.is_file():
            self.skipTest("target/debug/meetnote is not built")
        record = json.loads(CORPUS.read_text(encoding="utf-8").splitlines()[0])
        public_record = {key: value for key, value in record.items() if key not in {"gold", "negativeCase"}}
        prediction = {
            "scenarioId": public_record["scenarioId"],
            "mutations": [{
                "op": "set_title",
                "value": "External organize title",
                "evidence_segment_ids": [
                    public_record["record"]["state"]["results"]["transcript"][0]["id"]
                ],
            }],
            "rationale": "Model-generated explanation for the title change.",
            "semanticRationale": {"evidenceSegmentIds": [
                public_record["record"]["state"]["results"]["transcript"][0]["id"]
            ]},
        }
        with tempfile.TemporaryDirectory(prefix="assistant-eval-organize-test-") as temporary:
            evidence = writeback_prediction(
                prediction,
                public_record,
                WritebackConfig(cli=[str(MEETNOTE)], package_root=Path(temporary)),
                family="organize",
                run_key="organize-typed-receipt",
            )
        self.assertEqual(len(evidence["mutationReceipts"]), 1)
        self.assertTrue(evidence["mutationReceipts"][0]["mutationId"])
        self.assertEqual(evidence["readback"][0]["metadata"]["revision"], 1)
        self.assertEqual(evidence["readback"][0]["metadata"]["title"], "External organize title")


if __name__ == "__main__":
    unittest.main()
