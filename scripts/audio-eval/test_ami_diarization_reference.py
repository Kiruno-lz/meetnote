import importlib.metadata
import contextlib
import hashlib
import io
import json
from unittest import mock
import subprocess
import sys
import tempfile
import unittest
import wave
from array import array
from pathlib import Path


def available():
    try:
        return importlib.metadata.version("pyannote.core") == "6.1.0"
    except importlib.metadata.PackageNotFoundError:
        return False


@unittest.skipUnless(available(), "requires requirements-diarization.txt")
class AmiDiarizationReferenceTests(unittest.TestCase):
    def test_clipped_reference_preserves_overlap_and_original_speaker_identity(self):
        from prepare_ami_diarization import project_speakers
        from pyannote.core import Annotation, Segment, Timeline

        reference = Annotation()
        reference[Segment(0.5, 1.5), "first"] = "A"
        reference[Segment(1, 2), "second"] = "B"
        regions = Timeline([Segment(0, 3)])
        self.assertEqual(
            project_speakers(reference, regions, 16000, 28000),
            [
                {"startMs": 0, "endMs": 500, "speaker": "A"},
                {"startMs": 0, "endMs": 750, "speaker": "B"},
            ],
        )

    def test_an_unannotated_gap_cannot_be_replaced_with_a_whole_file_region(self):
        from prepare_ami_diarization import project_speakers
        from pyannote.core import Annotation, Segment, Timeline

        regions = Timeline([Segment(0, 0.5), Segment(1, 2)])
        with self.assertRaisesRegex(ValueError, "UEM"):
            project_speakers(Annotation(), regions, 0, 24000)
        for first, last in [(0, 0), (-1, 100), (100, 99), (True, 200)]:
            with self.assertRaises(ValueError):
                project_speakers(Annotation(), regions, first, last)

    def test_audio_crop_must_match_the_declared_offset_and_pcm_format(self):
        from prepare_ami_diarization import verify_crop

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            samples = array("h", (i % 1000 for i in range(4000)))

            def write(path, values, rate=16000):
                with wave.open(str(path), "wb") as w:
                    w.setnchannels(1)
                    w.setsampwidth(2)
                    w.setframerate(rate)
                    w.writeframes(values.tobytes())

            source, crop = root / "source.wav", root / "crop.wav"
            write(source, samples)
            write(crop, samples[1000:3000])
            self.assertEqual(verify_crop(source, crop, 1000), 3000)
            with self.assertRaisesRegex(ValueError, "PCM"):
                verify_crop(source, crop, 1100)
            write(crop, samples[1000:3000], 8000)
            with self.assertRaisesRegex(ValueError, "16 kHz"):
                verify_crop(source, crop, 1000)

    def test_fixture_producer_binds_pcm_reference_and_partition_without_changing_inputs(self):
        import prepare_ami_diarization as prepare

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, crop = root / "source.wav", root / "crop.wav"
            samples = array("h", (i % 1000 for i in range(32000)))
            for path, pcm in [(source, samples), (crop, samples[8000:24000])]:
                with wave.open(str(path), "wb") as w:
                    w.setnchannels(1)
                    w.setsampwidth(2)
                    w.setframerate(16000)
                    w.writeframes(pcm.tobytes())
            rttm, uem, split = root / "reference.rttm", root / "reference.uem", root / "train.txt"
            rttm.write_text(
                "SPEAKER fixture 1 0.25 1.0 <NA> <NA> A <NA> <NA>\nSPEAKER fixture 1 1.0 0.5 <NA> <NA> B <NA> <NA>\n"
            )
            uem.write_text("fixture 1 0.0 2.0\n")
            split.write_text("fixture\n")
            before = {path: path.read_bytes() for path in [source, crop, rttm, uem, split]}
            digest = lambda path: hashlib.sha256(before[path]).hexdigest()
            argv = [
                "prepare",
                "--source-audio",
                str(source),
                "--audio",
                str(crop),
                "--rttm",
                str(rttm),
                "--uem",
                str(uem),
                "--split-file",
                str(split),
                "--start-ms",
                "500",
                "--root",
                str(root),
            ]
            # Fixture-only hashes exercise publication without a production hash-bypass option.
            with mock.patch.multiple(
                prepare,
                MEETING="fixture",
                REVISION="test-fixture",
                AUDIO_SHA=digest(source),
                RTTM_SHA=digest(rttm),
                UEM_SHA=digest(uem),
                SPLIT_SHA=digest(split),
            ), mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
                prepare.main()
            record = json.loads((root / "diarization-reference.json").read_text())
            self.assertEqual(record["firstSourceSample"], 8000)
            self.assertEqual(record["lastSourceSample"], 24000)
            self.assertEqual(record["audioSha256"], digest(crop))
            self.assertEqual(record["split"], "train")
            self.assertEqual(record["scoringRegionsMs"], [[0, 1000]])
            self.assertEqual(
                record["referenceSpeakerSegments"],
                [
                    {"startMs": 0, "endMs": 750, "speaker": "A"},
                    {"startMs": 500, "endMs": 1000, "speaker": "B"},
                ],
            )
            self.assertTrue(all(path.read_bytes() == content for path, content in before.items()))
            self.assertFalse(list(root.glob(".ami-reference-*")))

    def test_explicit_heldout_profile_preserves_source_and_partition_pins(self):
        import prepare_ami_diarization as prepare

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.wav"
            with wave.open(str(source), "wb") as stream:
                stream.setnchannels(1)
                stream.setsampwidth(2)
                stream.setframerate(16000)
                stream.writeframes(array("h", range(16000)).tobytes())
            rttm, uem, split = root / "reference.rttm", root / "reference.uem", root / "test.txt"
            rttm.write_text("SPEAKER heldout 1 0.25 0.5 <NA> <NA> A <NA> <NA>\n")
            uem.write_text("heldout 1 0.0 1.0\n")
            split.write_text("heldout\n")
            inputs = [source, rttm, uem, split]
            before = {path: path.read_bytes() for path in inputs}
            digest = lambda path: hashlib.sha256(before[path]).hexdigest()
            profile = {
                "split": "test",
                "audioSha256": digest(source),
                "rttmSha256": digest(rttm),
                "uemSha256": digest(uem),
                "splitSha256": digest(split),
            }
            argv = [
                "prepare",
                "--meeting",
                "heldout",
                "--source-audio",
                str(source),
                "--audio",
                str(source),
                "--rttm",
                str(rttm),
                "--uem",
                str(uem),
                "--split-file",
                str(split),
                "--root",
                str(root),
            ]
            output = root / "diarization-reference.json"
            with mock.patch.object(
                prepare, "HELDOUT_SOURCES", {"heldout": profile}, create=True
            ), mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
                prepare.main()
                record = json.loads(output.read_bytes())
                self.assertEqual(record["meetingId"], "heldout")
                self.assertEqual(record["split"], "test")
                self.assertEqual(record["sourceRttmPath"], "only_words/rttms/test/heldout.rttm")
                self.assertEqual(record["sourceUemPath"], "uems/test/heldout.uem")
                self.assertEqual(record["sourceSplitPath"], "lists/test.meetings.txt")
                self.assertEqual(record["sourceAudioSha256"], digest(source))
                self.assertEqual(
                    record["referenceSpeakerSegments"],
                    [{"startMs": 250, "endMs": 750, "speaker": "A"}],
                )
                preserved = output.read_bytes()
                for path in inputs:
                    with self.subTest(tampered=path.name):
                        path.write_bytes(before[path] + b"changed")
                        with self.assertRaisesRegex(ValueError, "checksum"):
                            prepare.main()
                        self.assertEqual(output.read_bytes(), preserved)
                        path.write_bytes(before[path])
                for field, path, value, message in [
                    ("splitSha256", split, b"different-meeting\n", "partition"),
                    (
                        "rttmSha256",
                        rttm,
                        b"SPEAKER other 1 0.25 0.5 <NA> <NA> A <NA> <NA>\n",
                        "identity",
                    ),
                ]:
                    with self.subTest(field=field):
                        path.write_bytes(value)
                        profile[field] = hashlib.sha256(value).hexdigest()
                        with self.assertRaisesRegex(ValueError, message):
                            prepare.main()
                        self.assertEqual(output.read_bytes(), preserved)
                        path.write_bytes(before[path])
                        profile[field] = digest(path)
            self.assertTrue(all(path.read_bytes() == data for path, data in before.items()))
            self.assertFalse(list(root.glob(".ami-reference-*")))

    def test_reference_output_cannot_alias_an_input(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.wav"
            source.write_bytes(b"protected source")
            (root / "diarization-reference.json").hardlink_to(source)
            result = subprocess.run(
                [
                    sys.executable,
                    str(Path(__file__).with_name("prepare_ami_diarization.py")),
                    "--source-audio",
                    str(source),
                    "--audio",
                    str(source),
                    "--rttm",
                    str(source),
                    "--uem",
                    str(source),
                    "--split-file",
                    str(source),
                    "--root",
                    str(root),
                ],
                capture_output=True,
                text=True,
                timeout=30,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("overwrite an input", result.stderr)
            self.assertEqual(source.read_bytes(), b"protected source")

    def test_unpinned_inputs_do_not_replace_an_existing_reference(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ["source.wav", "crop.wav", "reference.rttm", "reference.uem", "train.txt"]:
                (root / name).write_bytes(b"not a pinned corpus artifact")
            output = root / "diarization-reference.json"
            output.write_bytes(b"existing reference")
            result = subprocess.run(
                [
                    sys.executable,
                    str(Path(__file__).with_name("prepare_ami_diarization.py")),
                    "--source-audio",
                    str(root / "source.wav"),
                    "--audio",
                    str(root / "crop.wav"),
                    "--rttm",
                    str(root / "reference.rttm"),
                    "--uem",
                    str(root / "reference.uem"),
                    "--split-file",
                    str(root / "train.txt"),
                    "--root",
                    str(root),
                ],
                capture_output=True,
                text=True,
                timeout=30,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("checksum", result.stderr)
            self.assertEqual(output.read_bytes(), b"existing reference")


if __name__ == "__main__":
    unittest.main()
