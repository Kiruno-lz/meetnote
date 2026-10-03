import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

from dataset_eval import (
    DATASET_FILES,
    DatasetEvalError,
    _runner_status,
    assemble_boundary_predictions,
    build_boundary_request,
    build_request,
    load_records,
    public_record,
    quality_thresholds,
    run_dataset,
    run_boundary_first_dataset,
    split_boundary_windows,
)


ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "scripts/assistant-eval/runner.py"


def _record(scenario_id: str = "dataset-scenario", mode: str = "partition") -> dict:
    segments = [
        {"id": "seg-1", "startMs": 0, "endMs": 1000, "speaker": "A", "text": "one topic"},
        {"id": "seg-2", "startMs": 1000, "endMs": 2000, "speaker": "A", "text": "same topic"},
    ]
    return {
        "schemaVersion": 1,
        "scenarioId": scenario_id,
        "segments": segments,
        "gold": {
            "allowed_mode": mode,
            "children": [
                {
                    "start_segment_id": "seg-1",
                    "end_segment_id": "seg-2" if mode == "partition" else "seg-1",
                    "evidence_segment_ids": ["seg-1"],
                    "title": ["one topic"],
                    "tags": [["topic"]],
                    "summary": ["one topic"],
                }
            ],
            "uncovered_ranges": []
            if mode == "partition"
            else [{"start_segment_id": "seg-2", "end_segment_id": "seg-2"}],
        },
    }


class DatasetEvalTests(unittest.TestCase):
    def test_boundary_windows_overlap_and_cover_each_adjacent_owner(self):
        record = {"segments": [{"id": f"seg-{index}"} for index in range(130)]}
        windows = split_boundary_windows(record)
        self.assertEqual(windows[0], (0, 64))
        self.assertEqual(windows[1][0], 56)
        self.assertEqual(windows[-1][1], 130)
        owned_boundaries = set()
        for start, end in windows:
            owned_boundaries.update(range(start, end - 1))
        self.assertEqual(owned_boundaries, set(range(129)))

    def test_boundary_first_request_and_assembler_are_public_and_deterministic(self):
        request, _ = build_boundary_request(
            "ami", "openaiResponses", [_record(mode="partition")]
        )
        self.assertEqual(request["taskFamily"], "boundary")
        self.assertEqual(request["outputContract"]["required"], ["scenarioId", "transitions"])
        self.assertNotIn("gold", json.dumps(request["records"]))
        self.assertIn("adjacent pair", request["prompt"]["task"])
        assembled = assemble_boundary_predictions(
            [_record(mode="partition")],
            [{"scenarioId": "dataset-scenario", "transitions": [{"after_segment_id": "seg-1", "boundary": False}]}],
        )
        self.assertEqual(len(assembled[0]["children"]), 1)
        self.assertEqual(assembled[0]["children"][0]["start_segment_id"], "seg-1")
        self.assertEqual(assembled[0]["children"][0]["end_segment_id"], "seg-2")

    def test_boundary_first_runner_report_keeps_raw_and_assembled_hashes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            data = root / "ami.jsonl"
            data.write_text(json.dumps(_record(mode="partition")) + "\n", encoding="utf-8")
            fake = root / "fake_boundary_runner.py"
            fake.write_text(
                "import json, sys\n"
                "request = json.load(sys.stdin)\n"
                "predictions = [{'scenarioId': record['scenarioId'], 'transitions': [{'after_segment_id': record['segments'][0]['id'], 'boundary': False}]} for record in request['records']]\n"
                "print(json.dumps({'status': 'completed', 'predictions': predictions, 'metadata': {'predictionSource': 'external_runner'}}))\n",
                encoding="utf-8",
            )
            report = run_boundary_first_dataset(
                data,
                protocol="openaiResponses",
                runner=f"{sys.executable} {fake}",
                limit=1,
                environment={},
                timeout=10,
                output_dir=root / "out",
            )
            self.assertTrue(report["boundaryFirst"])
            self.assertEqual(report["status"], "completed")
            self.assertEqual(report["qualityStatus"], "passed")
            self.assertTrue(report["predictionSha256"])
            self.assertTrue(report["rawBoundaryPredictionSha256"])
            self.assertNotEqual(report["predictionSha256"], report["rawBoundaryPredictionSha256"])

    def test_model_rationale_is_allowed_but_annotation_fields_are_rejected(self):
        status, predictions, _, _ = _runner_status(
            {
                "status": "completed",
                "predictions": [
                    {
                        "scenarioId": "dataset-scenario",
                        "mode": "extract",
                        "children": [{"rationale": "model-generated evidence explanation"}],
                        "uncovered_ranges": [],
                    }
                ],
            }
        )
        self.assertEqual(status, "completed")
        self.assertEqual(predictions[0]["children"][0]["rationale"], "model-generated evidence explanation")
        with self.assertRaises(DatasetEvalError):
            _runner_status(
                {
                    "status": "completed",
                    "predictions": [
                        {
                            "scenarioId": "dataset-scenario",
                            "mode": "extract",
                            "children": [{"gold": {"children": []}}],
                            "uncovered_ranges": [],
                        }
                    ],
                }
            )

    def test_dataset_task_semantics_match_converters(self):
        qmsum_request, _ = build_request(
            "qmsum",
            "openaiResponses",
            [_record(mode="extract")],
            single_topic_only=True,
        )
        ami_request, _ = build_request(
            "ami",
            "openaiResponses",
            [_record(mode="partition")],
            single_topic_only=True,
        )
        self.assertEqual(qmsum_request["taskFamily"], "extract")
        self.assertEqual(qmsum_request["outputContract"]["expectedMode"], "extract")
        self.assertEqual(qmsum_request["budget"]["maxOutputTokens"], 4096)
        self.assertEqual(qmsum_request["resources"]["maxOutputTokens"], 4096)
        self.assertIn("uncovered_ranges", qmsum_request["prompt"]["task"])
        self.assertEqual(ami_request["taskFamily"], "partition")
        self.assertEqual(ami_request["outputContract"]["expectedMode"], "partition")
        ami_task = ami_request["prompt"]["task"]
        self.assertIn("topic object, decision, task", ami_task)
        self.assertIn("same speaker continues", ami_task)
        self.assertIn("new agenda item", ami_task)
        self.assertIn("chair or facilitator transition", ami_task)
        self.assertIn("new presenter or demonstration stage", ami_task)
        self.assertIn("task or action-item assignment", ami_task)
        self.assertIn("meeting closing or wrap-up", ami_task)
        self.assertIn("scan every adjacent turn", ami_task)
        self.assertIn("final JSON contains", ami_task)
        self.assertIn("never infer boundaries", ami_task)
        self.assertIn("gold label", ami_task)
        self.assertEqual(quality_thresholds(True)["boundaryF1"], 0.0)
        self.assertEqual(quality_thresholds(False)["boundaryF1"], 0.8)

    def test_public_projection_removes_gold_and_annotation_answers(self):
        record = _record()
        record.update(
            {
                "queries": [{"query": "private", "answer": "gold", "relevant_text_span": [[0, 1]]}],
                "topicHierarchy": {"nestedTopics": [{"title": "private annotation"}]},
            }
        )
        public = public_record(record)
        encoded = json.dumps(public, ensure_ascii=False)
        self.assertNotIn("gold", encoded)
        self.assertNotIn("private", encoded)
        self.assertEqual([segment["id"] for segment in public["segments"]], ["seg-1", "seg-2"])

    def test_external_prediction_is_scored_against_both_baselines(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            data = root / "qmsum.jsonl"
            data.write_text(json.dumps(_record(mode="extract"), ensure_ascii=False) + "\n", encoding="utf-8")
            capture = root / "request.json"
            fake = root / "fake_runner.py"
            fake.write_text(
                "import json, os, sys\n"
                "request = json.load(sys.stdin)\n"
                "open(os.environ['CAPTURE'], 'w', encoding='utf-8').write(json.dumps(request))\n"
                "predictions = []\n"
                "for record in request['records']:\n"
                "    segments = record['segments']\n"
                "    predictions.append({'scenarioId': record['scenarioId'], 'mode': 'extract', 'children': [{'start_segment_id': segments[0]['id'], 'end_segment_id': segments[0]['id'], 'evidence_segment_ids': [segments[0]['id']]}], 'uncovered_ranges': [{'start_segment_id': segments[1]['id'], 'end_segment_id': segments[1]['id']}]})\n"
                "print(json.dumps({'status': 'completed', 'predictions': predictions, 'metadata': {'predictionSource': 'external_runner'}}))\n",
                encoding="utf-8",
            )
            report = run_dataset(
                "qmsum",
                data,
                protocol="openaiResponses",
                runner=f"{sys.executable} {fake}",
                limit=1,
                single_topic_only=True,
                environment={"CAPTURE": str(capture)},
                timeout=10,
                output_dir=root / "out",
            )
            self.assertEqual(report["status"], "completed")
            self.assertEqual(report["qualityStatus"], "passed")
            self.assertTrue(report["singleTopicCheck"]["passed"])
            self.assertEqual(report["thresholds"]["boundaryF1"], 0.0)
            self.assertIn("outperforms", report["baselineComparison"])
            request = json.loads(capture.read_text(encoding="utf-8"))
            encoded = json.dumps(request["records"], ensure_ascii=False)
            self.assertNotIn('"gold":', encoded)
            self.assertNotIn("relevant_text_span", encoded)
            self.assertNotIn("topicHierarchy", encoded)
            self.assertFalse(request["holdout"]["goldIncluded"])

    def test_missing_external_configuration_is_not_assessed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            data = root / "qmsum.jsonl"
            data.write_text(json.dumps(_record(mode="extract"), ensure_ascii=False) + "\n", encoding="utf-8")
            report = run_dataset(
                "qmsum",
                data,
                protocol="openaiResponses",
                runner=f"{sys.executable} {RUNNER}",
                limit=1,
                single_topic_only=False,
                environment={},
                timeout=10,
                output_dir=root / "out",
            )
            self.assertEqual(report["status"], "not_assessed")
            self.assertNotIn("evaluation", report)
            prediction = (root / "out" / "prediction.jsonl").read_text(encoding="utf-8")
            self.assertEqual(prediction, "")

    def test_existing_converted_qmsum_and_ami_are_publicly_projectable(self):
        for dataset in ("qmsum", "ami"):
            path = DATASET_FILES[dataset]
            if not path.is_file():
                self.skipTest(f"converted {dataset} fixture is not present")
            records = load_records(path, limit=1)
            public = public_record(records[0])
            self.assertNotIn("gold", json.dumps(public))
            self.assertTrue(public["segments"])


if __name__ == "__main__":
    unittest.main()
