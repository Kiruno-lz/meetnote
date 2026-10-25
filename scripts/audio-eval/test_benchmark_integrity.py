import hashlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

from asr_benchmark import load_benchmark_run as load_run

SCRIPTS = Path(__file__).parent.resolve()


class BenchmarkIntegrityTests(unittest.TestCase):
    def make_run(self, root, repeats=2):
        run = root / "runs" / "case"
        run.mkdir(parents=True)
        audio = root / "fixture.wav"
        audio.write_bytes(b"fixture")
        cases = [
            {
                "id": "a",
                "audioPath": str(audio),
                "sha256": hashlib.sha256(audio.read_bytes()).hexdigest(),
                "durationSeconds": 1,
                "reference": "gold",
                "language": "en",
            }
        ]
        case_path = run / "cases.json"
        case_path.write_text(json.dumps(cases))
        model = root / "model.bin"
        model.write_bytes(b"fixture")
        config = {
            "model": {"backendId": "fixture", "artifactPath": str(model)},
            "threads": 1,
            "repeats": repeats,
        }
        (run / "config.json").write_text(json.dumps(config))
        report = {
            "failure": None,
            "config": config,
            "casesSha256": hashlib.sha256(case_path.read_bytes()).hexdigest(),
            "runnerSha256": "fixture",
            "peakRssBytes": None,
            "modelLoading": "per-call",
        }
        (run / "report.json").write_text(json.dumps(report))
        row = {
            "caseId": "a",
            "repeat": 0,
            "status": "ok",
            "text": "gold",
            "wallSeconds": 0.1,
            "transcript": [],
        }
        (run / "prediction.jsonl").write_text(json.dumps(row) + "\n")
        return run

    def test_missing_entire_repeat_is_rejected_by_gate_and_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            run = self.make_run(root)
            with self.assertRaisesRegex(ValueError, "coverage|repeat|incomplete"):
                load_run(run)

    def test_window_gate_rejects_an_entire_missing_repeat(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            run = self.make_run(root)
            case = json.loads((run / "cases.json").read_text())[0]
            cases = [
                dict(case, scenario="composed-clean"),
                dict(case, id="b", scenario="composed-noise20db"),
            ]
            case_path = run / "cases.json"
            case_path.write_text(json.dumps(cases))
            report = json.loads((run / "report.json").read_text())
            report["casesSha256"] = hashlib.sha256(case_path.read_bytes()).hexdigest()
            (run / "report.json").write_text(json.dumps(report))
            rows = [
                {
                    "caseId": c["id"],
                    "repeat": 0,
                    "status": "ok",
                    "text": "gold",
                    "wallSeconds": 0.1,
                    "transcript": [{"startMs": 0, "endMs": 1000, "text": "gold"}],
                }
                for c in cases
            ]
            (run / "prediction.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "check_asr_windows.py"),
                    "--run",
                    str(run),
                    "--language",
                    "en",
                    "--max-clean",
                    ".2",
                    "--max-noise",
                    ".25",
                ],
                capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("coverage", result.stderr.decode())

    def test_report_rejects_a_missing_entire_repeat(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.make_run(root)
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "report_asr.py"),
                    "--root",
                    str(root),
                    "--output",
                    str(root / "report"),
                ],
                capture_output=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((root / "report/asr-comparison.json").exists())

    def test_duplicate_and_out_of_range_repeats_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = self.make_run(Path(tmp))
            first = json.loads((run / "prediction.jsonl").read_text())
            for repeat in (0, -1, 2):
                with self.subTest(repeat=repeat):
                    rows = [first, dict(first, repeat=repeat)]
                    (run / "prediction.jsonl").write_text(
                        "".join(json.dumps(row) + "\n" for row in rows)
                    )
                    with self.assertRaises(ValueError):
                        load_run(run)

    def test_new_success_receipt_detects_replaced_predictions(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            run = self.make_run(root, repeats=1)
            fake = root / "success.py"
            fake.write_text(
                "import json,sys\nfrom pathlib import Path\n"
                'row={"caseId":"a","repeat":0,"status":"ok","text":"gold","wallSeconds":.1}\n'
                'Path(sys.argv[3]).write_text(json.dumps(row)+"\\n")\n'
            )
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPTS / "asr_benchmark.py"),
                    "--cases",
                    str(run / "cases.json"),
                    "--config",
                    str(run / "config.json"),
                    "--runner",
                    str(fake),
                    "--output",
                    str(run),
                ],
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            report, _, rows = load_run(run)
            self.assertIn("predictionSha256", report)
            (run / "prediction.jsonl").write_text(json.dumps(dict(rows[0], text="replaced")) + "\n")
            with self.assertRaisesRegex(ValueError, "checksum"):
                load_run(run)



if __name__ == "__main__":
    unittest.main()
