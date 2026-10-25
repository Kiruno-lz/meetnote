import copy
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from check_diarization_cpu import compare_runs


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
