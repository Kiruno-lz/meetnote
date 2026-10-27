import contextlib
import io
import json
import sys
import tempfile
import unittest
import wave
import zipfile
import xml.etree.ElementTree as ET
from pathlib import Path
from unittest import mock

import prepare_ami_asr as ami


class AmiReferenceTests(unittest.TestCase):
    @staticmethod
    def word(identity="w", start="1.25", end="1.75", text="hello"):
        word = ET.Element("w", {ami.NITE + "id": identity, "starttime": start, "endtime": end})
        word.text = text
        return word

    def test_projection_is_sample_based_and_retains_overlapping_word_intervals(self):
        timing = ami.project_reference_words(
            [
                self.word("a", "1.00003125", "1.00009375"),
                self.word("b", "1.25", "1.75"),
                self.word("c", "1.5", "1.8"),
            ],
            16000,
            32000,
        )
        self.assertEqual(
            [(w["startSample"], w["endSample"]) for w in timing["words"]],
            [(0, 2), (4000, 12000), (8000, 12800)],
        )

    def test_projection_rejects_bad_partial_and_unordered_words_without_clipping(self):
        for words in [
            [],
            [self.word(start="NaN")],
            [self.word(end="Infinity")],
            [self.word(start="not-a-time")],
            [self.word(start="-1")],
            [self.word(start="1.75", end="1.25")],
            [self.word(end="1.25")],
            [self.word(start="0.999999")],
            [self.word(end="2.000001")],
            [self.word(start="1.00001", end="1.00002")],
            [self.word(text=" ")],
            [self.word(identity="")],
            [self.word(), self.word()],
            [self.word("later", "1.5", "1.6"), self.word("earlier", "1.2", "1.3")],
        ]:
            with self.subTest(words=[w.attrib for w in words]), self.assertRaises(ValueError):
                ami.project_reference_words(words, 16000, 32000)
        missing = self.word()
        del missing.attrib["endtime"]
        with self.assertRaisesRegex(ValueError, "w"):
            ami.project_reference_words([missing], 16000, 32000)

    def test_preparer_preserves_words_and_projects_official_times_to_crop_samples(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            audio = root / "source.wav"
            annotations = root / "manual.zip"
            output = root / "prepared"
            with wave.open(str(audio), "wb") as wav:
                wav.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
                wav.writeframes(bytes(21 * 32000))
            with zipfile.ZipFile(annotations, "w") as archive:
                for i, speaker in enumerate("ABCD"):
                    words = "".join(
                        f'<w nite:id="{speaker}{j}" starttime="{1+i*5+j/2}" '
                        f'endtime="{1+i*5+(j+1)/2}">word{j}</w>'
                        for j in range(6)
                    )
                    archive.writestr(
                        f"words/ES2002a.{speaker}.words.xml",
                        f'<root xmlns:nite="http://nite.sourceforge.net/">{words}</root>',
                    )
                    archive.writestr(
                        f"segments/ES2002a.{speaker}.segments.xml",
                        f'<root><segment><child href="words#id({speaker}0)..id({speaker}5)"/></segment></root>',
                    )
            with mock.patch.object(ami, "AUDIO_SHA", ami.sha256(audio)), mock.patch.object(
                ami, "ANNOTATION_SHA", ami.sha256(annotations)
            ), mock.patch.object(
                sys,
                "argv",
                [
                    "prepare_ami_asr",
                    "--audio",
                    str(audio),
                    "--annotations",
                    str(annotations),
                    "--root",
                    str(output),
                ],
            ), contextlib.redirect_stdout(
                io.StringIO()
            ):
                ami.main()
            cases = json.loads((output / "ami-cases.json").read_text())
            self.assertEqual(len(cases), 4)
            case = cases[0]
            self.assertEqual(case["reference"], "word0 word1 word2 word3 word4 word5")
            timing = case["wordTiming"]
            self.assertEqual(timing["source"], "ami-manual-1.6.2-forced-alignment")
            self.assertFalse(timing["manuallyVerified"])
            self.assertEqual(timing["sampleRateHz"], 16000)
            self.assertEqual(
                timing["words"][0],
                {
                    "id": "A0",
                    "text": "word0",
                    "startSample": 1600,
                    "endSample": 9600,
                },
            )
            self.assertEqual(timing["words"][-1]["endSample"], 49600)
            self.assertEqual([word["id"] for word in timing["words"]], case["sourceWordIds"])
            self.assertNotIn("knownSilenceRangesMs", case)


if __name__ == "__main__":
    unittest.main()
