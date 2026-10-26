import contextlib
import hashlib
import importlib.metadata
import io
import json
import subprocess
import sys
import tempfile
import unittest
import wave
from pathlib import Path
from unittest import mock


def available():
    try:
        return importlib.metadata.version("pyannote.metrics") == "4.0.0"
    except importlib.metadata.PackageNotFoundError:
        return False


@unittest.skipUnless(available(), "requires requirements-diarization.txt")
class DiarizationQualityTests(unittest.TestCase):
    def fixture(self, root, empty=False):
        run = root / "run"
        run.mkdir()
        audio = root / "fixture.wav"
        with wave.open(str(audio), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(16000)
            w.writeframes(bytes(32000))
        digest = lambda b: hashlib.sha256(b).hexdigest()
        reference = {
            "schema": "meetnote.diarization-reference.v1",
            "meetingId": "fixture",
            "dataset": "synthetic-protocol-fixture",
            "split": "fixture",
            "referenceVariant": "synthetic",
            "referenceTiming": "hand-calculated test annotations, no model execution",
            "sourceRepository": "fixture",
            "sourceRevision": "fixture-only",
            "sourceRttmSha256": digest(b"rttm fixture"),
            "sourceUemSha256": digest(b"uem fixture"),
            "audioPath": str(audio),
            "audioSha256": digest(audio.read_bytes()),
            "durationMs": 1000,
            "sampleRateHz": 16000,
            "firstSourceSample": 0,
            "lastSourceSample": 16000,
            "scoringRegionsMs": [[0, 1000]],
            "qualityAcceptance": "not_assessed",
            "referenceSpeakerSegments": [{"startMs": 0, "endMs": 1000, "speaker": "A"}],
        }
        ref = root / "reference.json"
        ref.write_text(json.dumps(reference))
        segments = [] if empty else [{"speaker_id": "X", "start_ms": 0, "end_ms": 1000}]
        rows = [
            {
                "repeat": i,
                "status": "ok",
                "wallSeconds": 0.1,
                "backend": "diarization-native",
                "speakerCount": len(segments),
                "segments": len(segments),
                "speakerSegments": segments,
                "observations": 0,
                "embeddingObservations": [],
            }
            for i in range(2)
        ]
        report = {
            "schema": "meetnote.diarization-cpu-run.v1",
            "audioSha256": reference["audioSha256"],
            "audioSeconds": 1,
            "modelArtifacts": {"fixture.model": digest(b"model")},
            "runnerSha256": digest(b"runner"),
            "configuration": {"repeats": 2, "backend": "diarization-native", "device": "Cpu"},
            "failure": None,
            "peakRssBytes": 1000,
        }
        self.save(run, report, rows)
        return run, ref, report, rows

    def save(self, run, report, rows):
        payload = "".join(json.dumps(row) + "\n" for row in rows).encode()
        (run / "prediction.jsonl").write_bytes(payload)
        report["predictionSha256"] = hashlib.sha256(payload).hexdigest()
        (run / "report.json").write_text(json.dumps(report))

    def test_perfect_and_confused_outputs_have_real_der_without_claiming_acceptance(self):
        from report_diarization import evaluate_run

        with tempfile.TemporaryDirectory() as directory:
            run, ref, report, rows = self.fixture(Path(directory))
            result = evaluate_run(run, ref, [0])
            self.assertEqual([x["der"] for x in result["results"]], [0, 0])
            self.assertEqual(result["qualityAcceptance"], "not_assessed")
            self.assertEqual(result["asrQualityStatus"], "not_assessed")
            for row in rows:
                row["speakerSegments"] = [
                    {"speaker_id": "X", "start_ms": 0, "end_ms": 500},
                    {"speaker_id": "Y", "start_ms": 500, "end_ms": 1000},
                ]
                row["segments"] = 2
                row["speakerCount"] = 2
            self.save(run, report, rows)
            result = evaluate_run(run, ref, [0])
            self.assertEqual([x["der"] for x in result["results"]], [0.5, 0.5])

    def test_fractional_seconds_match_a_valid_whole_millisecond_source(self):
        from report_diarization import evaluate_run

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run, ref, report, rows = self.fixture(root)
            audio = root / "fixture.wav"
            with wave.open(str(audio), "wb") as w:
                w.setnchannels(1)
                w.setsampwidth(2)
                w.setframerate(16000)
                w.writeframes(bytes(16016 * 2))
            digest = hashlib.sha256(audio.read_bytes()).hexdigest()
            r = json.loads(ref.read_text())
            r.update(
                audioSha256=digest,
                durationMs=1001,
                lastSourceSample=16016,
                scoringRegionsMs=[[0, 1001]],
            )
            ref.write_text(json.dumps(r))
            report.update(audioSha256=digest, audioSeconds=1.001)
            self.save(run, report, rows)
            self.assertEqual(evaluate_run(run, ref, [0])["results"][0]["der"], 0)

    def test_explicit_empty_hypothesis_is_a_miss_not_missing_execution(self):
        from report_diarization import evaluate_run

        with tempfile.TemporaryDirectory() as directory:
            run, ref, _, _ = self.fixture(Path(directory), empty=True)
            result = evaluate_run(run, ref, [0])
            self.assertEqual([x["der"] for x in result["results"]], [1, 1])

    def test_wrong_audio_duration_region_and_incomplete_runs_are_rejected(self):
        from report_diarization import evaluate_run

        for mode in ["audio", "duration", "region", "repeat", "prediction-hash", "failed"]:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                run, ref, report, rows = self.fixture(Path(directory))
                r = json.loads(ref.read_text())
                if mode == "audio":
                    r["audioSha256"] = "0" * 64
                elif mode == "duration":
                    r["durationMs"] = 2000
                elif mode == "region":
                    r["scoringRegionsMs"] = [[0, 300], [500, 1000]]
                elif mode == "repeat":
                    rows.pop()
                elif mode == "failed":
                    report["failure"] = "timeout"
                self.save(run, report, rows)
                ref.write_text(json.dumps(r))
                if mode == "prediction-hash":
                    (run / "prediction.jsonl").write_text("{}\n")
                with self.assertRaises(ValueError):
                    evaluate_run(run, ref, [0])

    def test_cpu_and_backend_provenance_cannot_be_mislabeled(self):
        from report_diarization import evaluate_run

        for mode in ["device", "backend"]:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                run, ref, report, rows = self.fixture(Path(directory))
                if mode == "device":
                    report["configuration"]["device"] = "Metal"
                else:
                    rows[0]["backend"] = "sherpa-onnx"
                self.save(run, report, rows)
                with self.assertRaisesRegex(ValueError, "CPU|backend"):
                    evaluate_run(run, ref, [0])

    def test_loaded_report_fingerprint_does_not_follow_a_later_external_edit(self):
        import report_diarization as scorer

        with tempfile.TemporaryDirectory() as directory:
            run, ref, _, _ = self.fixture(Path(directory))
            source = run / "report.json"
            before = source.read_bytes()
            original = scorer.load_run

            def edit_after_load(*args, **kwargs):
                result = original(*args, **kwargs)
                source.write_bytes(before + b"\n")
                return result

            with mock.patch.object(scorer, "load_run", side_effect=edit_after_load):
                result = scorer.evaluate_run(run, ref, [0])
            self.assertEqual(result["inputReportSha256"], hashlib.sha256(before).hexdigest())

    def test_cli_clears_a_stale_report_but_protects_every_input_and_audio(self):
        script = Path(__file__).with_name("report_diarization.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run, ref, report, rows = self.fixture(root)
            audio = root / "fixture.wav"
            for output in [ref, run / "report.json", run / "prediction.jsonl", audio]:
                before = output.read_bytes()
                result = subprocess.run(
                    [
                        sys.executable,
                        str(script),
                        "--run",
                        str(run),
                        "--reference",
                        str(ref),
                        "--output",
                        str(output),
                    ],
                    capture_output=True,
                    text=True,
                    timeout=30,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(output.read_bytes(), before)
            output = root / "quality.json"
            output.write_text(
                json.dumps({"schema": "meetnote.diarization-quality.v1", "status": "complete"})
            )
            ref.write_text("not json")
            result = subprocess.run(
                [
                    sys.executable,
                    str(script),
                    "--run",
                    str(run),
                    "--reference",
                    str(ref),
                    "--output",
                    str(output),
                ],
                capture_output=True,
                text=True,
                timeout=30,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
