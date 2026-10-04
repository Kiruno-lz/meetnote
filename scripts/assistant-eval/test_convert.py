#!/usr/bin/env python3

import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from convert_ami import convert as convert_ami
from convert_qmsum import convert as convert_qmsum
from evaluate import read_jsonl, validate_gold_record


NITE = "http://nite.sourceforge.net/"


class ConverterTests(unittest.TestCase):
    def test_ami_drops_utterances_with_only_empty_word_text(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive_path = root / "ami.zip"
            with zipfile.ZipFile(archive_path, "w") as archive:
                archive.writestr(
                    "words/M1.A.words.xml",
                    f'''<nite:root xmlns:nite="{NITE}">
                    <w nite:id="M1.A.words0" starttime="0" endtime="1">kept</w>
                    <w nite:id="M1.A.words1" starttime="1" endtime="2"></w>
                    </nite:root>''',
                )
                archive.writestr(
                    "segments/M1.A.segments.xml",
                    f'''<nite:root xmlns:nite="{NITE}">
                    <segment nite:id="s1"><nite:child href="M1.A.words.xml#id(M1.A.words0)"/></segment>
                    <segment nite:id="s2"><nite:child href="M1.A.words.xml#id(M1.A.words1)"/></segment>
                    </nite:root>''',
                )
                archive.writestr(
                    "topics/M1.topic.xml",
                    f'''<nite:root xmlns:nite="{NITE}">
                    <topic nite:id="top-1" other_description="kept">
                      <nite:child href="M1.A.words.xml#id(M1.A.words0)"/>
                    </topic>
                    </nite:root>''',
                )
                archive.writestr("LICENCE.txt", "CC BY 4.0")
            output = root / "converted.jsonl"
            report = convert_ami(archive_path, output)
            converted = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(report["emptyTextUtterancesDropped"], 1)
            self.assertEqual(len(converted["segments"]), 1)
            self.assertEqual(converted["segments"][0]["text"], "kept")
            self.assertTrue(validate_gold_record(converted).valid)

    def test_qmsum_splits_discontinuous_topic_into_continuous_children(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            input_dir = root / "qmsum"
            input_dir.mkdir()
            source = {
                "meeting_transcripts": [
                    {"speaker": "A", "content": "alpha"},
                    {"speaker": "A", "content": "alpha detail"},
                    {"speaker": "B", "content": "unrelated"},
                    {"speaker": "A", "content": "alpha returns"},
                ],
                "topic_list": [
                    {"topic": "alpha", "relevant_text_span": [["0", "1"], ["3", "3"]]},
                    {"topic": "other", "relevant_text_span": [["2", "2"]]},
                ],
                "general_query_list": [],
                "specific_query_list": [],
            }
            (input_dir / "train.jsonl").write_text(
                json.dumps(source) + "\n", encoding="utf-8"
            )
            output = root / "converted.jsonl"
            report = convert_qmsum(input_dir, output)
            converted = read_jsonl(output)
            self.assertEqual(len(converted), 2)
            alpha_record = next(
                record for record in converted if record["source"]["topicIndex"] == 0
            )
            alpha = alpha_record["gold"]["children"]
            self.assertEqual(len(alpha), 2)
            self.assertNotEqual(
                alpha[0]["end_segment_id"], alpha[1]["start_segment_id"]
            )
            self.assertEqual(report["discontinuousTopicsSplit"], 1)
            self.assertEqual(report["unmappedSpans"], 0)
            self.assertEqual(report["license"], "MIT")
            self.assertTrue(
                all(validate_gold_record(record).valid for record in converted)
            )

    def test_ami_preserves_at_most_two_topic_levels(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive_path = root / "ami.zip"
            with zipfile.ZipFile(archive_path, "w") as archive:
                archive.writestr(
                    "words/M1.A.words.xml",
                    f'''<nite:root xmlns:nite="{NITE}" nite:id="M1.A.words">
                    <w nite:id="M1.A.words0" starttime="0" endtime="1">one</w>
                    <w nite:id="M1.A.words1" starttime="1" endtime="2">two</w>
                    <w nite:id="M1.A.words2" starttime="2" endtime="3">three</w>
                    <w nite:id="M1.A.words3" starttime="3" endtime="4">four</w>
                    </nite:root>''',
                )
                archive.writestr(
                    "segments/M1.A.segments.xml",
                    f'''<nite:root xmlns:nite="{NITE}" nite:id="M1.A.segs">
                    <segment nite:id="s0"><nite:child href="M1.A.words.xml#id(M1.A.words0)..id(M1.A.words1)"/></segment>
                    <segment nite:id="s1"><nite:child href="M1.A.words.xml#id(M1.A.words2)..id(M1.A.words3)"/></segment>
                    </nite:root>''',
                )
                archive.writestr(
                    "topics/M1.topic.xml",
                    f'''<nite:root xmlns:nite="{NITE}" nite:id="M1.topic">
                    <topic nite:id="top-1" other_description="first">
                      <topic nite:id="sub-1" other_description="sub"><nite:child href="M1.A.words.xml#id(M1.A.words0)..id(M1.A.words1)"/></topic>
                    </topic>
                    <topic nite:id="top-2" other_description="second"><nite:child href="M1.A.words.xml#id(M1.A.words2)..id(M1.A.words3)"/></topic>
                    </nite:root>''',
                )
                archive.writestr("LICENCE.txt", "CC BY 4.0")
            output = root / "converted.jsonl"
            report = convert_ami(archive_path, output)
            converted = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(report["maxTreeDepth"], 2)
            self.assertTrue(converted["topicHierarchy"]["nestedTopics"])
            self.assertTrue(
                all(
                    topic["depth"] == 2
                    for topic in converted["topicHierarchy"]["nestedTopics"]
                )
            )
            self.assertEqual(len(converted["gold"]["children"]), 2)
            self.assertEqual(report["license"], "CC-BY-4.0")
            self.assertTrue(validate_gold_record(converted).valid)


if __name__ == "__main__":
    unittest.main()
