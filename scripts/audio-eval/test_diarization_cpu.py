import copy
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from check_diarization_cpu import compare_runs, measure_runs, compare_idle_cpu, load_run


class DiarizationCpuTests(unittest.TestCase):
    def write_run(self, root, name, wall=100, cluster=0):
        path = root / name
        path.mkdir(exist_ok=True)
        row = {
            "repeat": 0,
            "status": "ok",
            "wallSeconds": wall,
            "speakerCount": 1,
            "segments": 1,
            "observations": 1,
            "speakerSegments": [
                {"speaker_id": f"speaker-{cluster}", "start_ms": 0, "end_ms": 1000}
            ],
            "embeddingObservations": [
                {
                    "clusterId": cluster,
                    "startMs": 0,
                    "endMs": 1000,
                    "quality": 0.5,
                    "embeddingSpace": {"dimension": 256},
                    "vector": [0.1] * 256,
                }
            ],
        }
        report = {
            "schema": "meetnote.diarization-cpu-run.v1",
            "audioSha256": "audio",
            "audioSeconds": 180,
            "modelArtifacts": {"model": "hash"},
            "configuration": {"repeats": 2, "device": "Cpu", "embeddingThreads": 1},
            "failure": None,
            "peakRssBytes": 1000,
            "runnerSha256": "runner",
        }
        self.save(path, report, [row, dict(copy.deepcopy(row), repeat=1)])
        return path

    def save(self, path, report, rows):
        content = "".join(json.dumps(row) + "\n" for row in rows).encode()
        (path / "prediction.jsonl").write_bytes(content)
        report["predictionSha256"] = hashlib.sha256(content).hexdigest()
        (path / "report.json").write_text(json.dumps(report))

    def edit(self, path, mutate):
        report = json.loads((path / "report.json").read_text())
        rows = [json.loads(s) for s in (path / "prediction.jsonl").read_text().splitlines()]
        mutate(report, rows)
        self.save(path, report, rows)

    def pair(self, root):
        return self.write_run(root, "baseline"), self.write_run(
            root, "candidate", wall=70, cluster=5
        )

    def test_permuted_ids_preserve_timeline_and_embeddings(self):
        with tempfile.TemporaryDirectory() as tmp:
            before, after = self.pair(Path(tmp))
            result = compare_runs(before, after)
            self.assertAlmostEqual(result["speedup"], 0.3)
            self.assertEqual(result["status"], "passed")

    def test_missing_and_duplicate_repeats_are_rejected(self):
        for mode in ("missing", "duplicate", "boolean"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                before, after = self.pair(Path(tmp))
                self.edit(
                    after,
                    lambda _, rows: (
                        rows.pop()
                        if mode == "missing"
                        else (
                            rows[0].update(repeat=False)
                            if mode == "boolean"
                            else rows[1].update(repeat=0)
                        )
                    ),
                )
                with self.assertRaisesRegex(ValueError, "repeat|coverage"):
                    compare_runs(before, after)

    def test_changed_inputs_and_replaced_prediction_are_rejected(self):
        for mode in ("audio", "model", "prediction"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                before, after = self.pair(Path(tmp))
                if mode == "prediction":
                    with (after / "prediction.jsonl").open("a") as stream:
                        stream.write("{}\n")
                else:
                    self.edit(
                        after,
                        lambda report, _: report.update(
                            {"audioSha256": "other"}
                            if mode == "audio"
                            else {"modelArtifacts": {"model": "other"}}
                        ),
                    )
                with self.assertRaises(ValueError):
                    compare_runs(before, after)

    def test_timing_change_is_rejected_even_with_the_same_counts(self):
        with tempfile.TemporaryDirectory() as tmp:
            before, after = self.pair(Path(tmp))
            self.edit(after, lambda _, rows: rows[0]["speakerSegments"][0].update(end_ms=1100))
            with self.assertRaisesRegex(ValueError, "timeline"):
                compare_runs(before, after)

    def test_unknown_embedding_cluster_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            before, after = self.pair(Path(tmp))
            self.edit(
                after, lambda _, rows: rows[0]["embeddingObservations"][0].update(clusterId=99)
            )
            with self.assertRaisesRegex(ValueError, "embedding origin"):
                compare_runs(before, after)

    def test_wrong_or_nonfinite_embeddings_are_rejected(self):
        for value in (-0.1, float("nan"), float("inf")):
            with self.subTest(value=value), tempfile.TemporaryDirectory() as tmp:
                before, after = self.pair(Path(tmp))
                self.edit(
                    after,
                    lambda _, rows: rows[0]["embeddingObservations"][0]["vector"].__setitem__(
                        0, value
                    ),
                )
                with self.assertRaisesRegex(ValueError, "embedding"):
                    compare_runs(before, after)

    def test_overflowing_embedding_norms_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            before, after = self.pair(Path(tmp))
            for path in (before, after):
                self.edit(
                    path,
                    lambda _, rows: rows[0]["embeddingObservations"][0]["vector"].__setitem__(
                        0, 1e308
                    ),
                )
            with self.assertRaisesRegex(ValueError, "embedding"):
                compare_runs(before, after)

    def test_boolean_wall_times_cannot_pass_as_a_speed_improvement(self):
        with tempfile.TemporaryDirectory() as tmp:
            before, after = self.pair(Path(tmp))
            self.edit(after, lambda _, rows: [row.update(wallSeconds=True) for row in rows])
            with self.assertRaisesRegex(ValueError, "measurement"):
                compare_runs(before, after)

    def test_quality_only_loading_preserves_the_strict_embedding_comparison_gate(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = self.write_run(Path(tmp), "empty")
            self.edit(
                run,
                lambda _, rows: [
                    row.update(
                        speakerSegments=[],
                        segments=0,
                        speakerCount=0,
                        embeddingObservations=[],
                        observations=0,
                    )
                    for row in rows
                ],
            )
            with self.assertRaisesRegex(ValueError, "missing"):
                load_run(run)
            _, rows = load_run(run, require_embeddings=False)
            self.assertEqual([row["speakerSegments"] for row in rows], [[], []])

    def test_measurement_does_not_confuse_equivalence_with_a_speed_gate(self):
        with tempfile.TemporaryDirectory() as tmp:
            before, after = self.pair(Path(tmp))
            self.edit(after, lambda _, rows: [row.update(wallSeconds=110) for row in rows])
            self.assertEqual(measure_runs(before, after)["qualityStatus"], "baseline_equivalent")
            with self.assertRaises(ValueError):
                compare_runs(before, after)

    def test_idle_cpu_gate_preserves_latency_and_thread_count(self):
        for mode in ("pass", "slow", "no_savings", "different_threads", "missing_cpu"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                before, after = self.pair(Path(tmp))
                self.edit(before, lambda report, _: report.update(cpuSeconds=400))
                self.edit(
                    after,
                    lambda report, rows: (
                        report.update(cpuSeconds=300),
                        report["configuration"].update(embeddingSpinning=False),
                        [row.update(wallSeconds=102) for row in rows],
                    ),
                )
                if mode == "slow":
                    self.edit(after, lambda _, rows: [row.update(wallSeconds=106) for row in rows])
                elif mode == "no_savings":
                    self.edit(after, lambda report, _: report.update(cpuSeconds=380))
                elif mode == "different_threads":
                    self.edit(
                        after, lambda report, _: report["configuration"].update(embeddingThreads=10)
                    )
                elif mode == "missing_cpu":
                    self.edit(after, lambda report, _: report.update(cpuSeconds=None))
                if mode == "pass":
                    self.assertEqual(compare_idle_cpu(before, after)["status"], "passed")
                else:
                    with self.assertRaises(ValueError):
                        compare_idle_cpu(before, after)

    def test_cli_output_cannot_replace_either_input_receipt_or_prediction(self):
        with tempfile.TemporaryDirectory() as tmp:
            before, after = self.pair(Path(tmp))
            for directory in (before, after):
                for name in ("report.json", "prediction.jsonl"):
                    output = directory / name
                    original = output.read_bytes()
                    command = [
                        sys.executable,
                        str(Path(__file__).with_name("check_diarization_cpu.py")),
                        "--baseline",
                        str(before),
                        "--candidate",
                        str(after),
                        "--output",
                        str(output),
                    ]
                    self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)
                    self.assertEqual(output.read_bytes(), original)

    def test_failed_cli_recheck_invalidates_previous_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            before, after = self.pair(root)
            output = root / "comparison.json"
            command = [
                sys.executable,
                str(Path(__file__).with_name("check_diarization_cpu.py")),
                "--baseline",
                str(before),
                "--candidate",
                str(after),
                "--output",
                str(output),
            ]
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 0)
            self.assertEqual(json.loads(output.read_text())["status"], "passed")
            self.edit(after, lambda _, rows: [row.update(wallSeconds=120) for row in rows])
            self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)
            self.assertFalse(output.exists(), "a failed check cannot leave a passing comparison")

    def test_insufficient_speed_and_excess_memory_are_rejected(self):
        for mode in ("speed", "memory"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                before, after = self.pair(Path(tmp))
                if mode == "memory":
                    self.edit(after, lambda report, _: report.update(peakRssBytes=1300))
                else:
                    self.edit(after, lambda _, rows: [row.update(wallSeconds=90) for row in rows])
                with self.assertRaises(ValueError):
                    compare_runs(before, after)


if __name__ == "__main__":
    unittest.main()
