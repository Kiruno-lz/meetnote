#!/usr/bin/env python3
"""Convert AMI manual words, utterances, and topic hierarchy into text examples."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import zipfile
from pathlib import Path
from typing import Any, Iterable
from xml.etree import ElementTree as ET

from fetch import DEFAULT_MANIFEST, verify_declared_files


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_INPUT = ROOT / "eval-data/assistant/data/ami/ami_public_manual_1.6.2.zip"
DEFAULT_OUTPUT = ROOT / "eval-data/assistant/data/converted/ami.jsonl"
NITE_ID = "{http://nite.sourceforge.net/}id"


def local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def href_ids(href: str, ordered_ids: dict[str, list[str]]) -> list[str]:
    filename, _, fragment = href.partition("#")
    matches = re.findall(r"id\(([^)]+)\)", fragment)
    if not matches:
        return []
    if len(matches) == 1:
        return matches
    ids = ordered_ids.get(filename, [])
    try:
        start, end = ids.index(matches[0]), ids.index(matches[1])
    except ValueError:
        return []
    return ids[start : end + 1] if start <= end else []


def parse_xml(archive: zipfile.ZipFile, member: str) -> ET.Element:
    return ET.fromstring(archive.read(member))


def words_for_meeting(
    archive: zipfile.ZipFile, meeting: str
) -> tuple[dict[str, dict[str, Any]], dict[str, list[str]]]:
    words: dict[str, dict[str, Any]] = {}
    ordered: dict[str, list[str]] = {}
    prefix = f"words/{meeting}."
    for member in sorted(
        name
        for name in archive.namelist()
        if name.startswith(prefix) and name.endswith(".words.xml")
    ):
        filename = Path(member).name
        speaker = filename.removeprefix(f"{meeting}.").removesuffix(".words.xml")
        ids = []
        for element in parse_xml(archive, member):
            word_id = element.attrib.get(NITE_ID)
            if not word_id:
                continue
            ids.append(word_id)
            try:
                start_ms = round(float(element.attrib["starttime"]) * 1_000)
                end_ms = round(float(element.attrib["endtime"]) * 1_000)
            except (KeyError, ValueError):
                start_ms = end_ms = None
            words[word_id] = {
                "id": word_id,
                "speaker": speaker,
                "startMs": start_ms,
                "endMs": end_ms,
                "text": (element.text or "").strip(),
            }
        ordered[filename] = ids
    return words, ordered


def utterances_for_meeting(
    archive: zipfile.ZipFile,
    meeting: str,
    words: dict[str, dict[str, Any]],
    ordered_ids: dict[str, list[str]],
) -> tuple[list[dict[str, Any]], int, int]:
    utterances = []
    unmapped = 0
    empty_text = 0
    prefix = f"segments/{meeting}."
    for member in sorted(
        name
        for name in archive.namelist()
        if name.startswith(prefix) and name.endswith(".segments.xml")
    ):
        filename = Path(member).name
        speaker = filename.removeprefix(f"{meeting}.").removesuffix(".segments.xml")
        for element in parse_xml(archive, member):
            if local_name(element.tag) != "segment":
                continue
            referenced = []
            for child_element in element:
                if local_name(child_element.tag) == "child":
                    referenced.extend(
                        href_ids(child_element.attrib.get("href", ""), ordered_ids)
                    )
            mapped = [
                words[word_id]
                for word_id in referenced
                if word_id in words and words[word_id]["startMs"] is not None
            ]
            if not mapped:
                unmapped += 1
                continue
            text = " ".join(word["text"] for word in mapped if word["text"])
            if not text.strip():
                empty_text += 1
                continue
            start_ms = min(word["startMs"] for word in mapped)
            end_ms = max(word["endMs"] for word in mapped)
            utterances.append(
                {
                    "sourceId": element.attrib.get(NITE_ID),
                    "speaker": speaker,
                    "startMs": start_ms,
                    "endMs": max(start_ms + 1, end_ms),
                    "text": text,
                    "wordIds": referenced,
                }
            )
    utterances.sort(
        key=lambda value: (value["startMs"], value["endMs"], value["sourceId"] or "")
    )
    for index, utterance in enumerate(utterances):
        utterance["id"] = f"ami-{meeting}-seg-{index:05d}"
    return utterances, unmapped, empty_text


def topic_word_ids(
    topic: ET.Element, ordered_ids: dict[str, list[str]], include_descendants: bool
) -> set[str]:
    result: set[str] = set()
    for child in topic:
        name = local_name(child.tag)
        if name == "child":
            result.update(href_ids(child.attrib.get("href", ""), ordered_ids))
        elif name == "topic" and include_descendants:
            result.update(topic_word_ids(child, ordered_ids, True))
    return result


def topic_title(topic: ET.Element) -> str:
    if topic.attrib.get("other_description"):
        return topic.attrib["other_description"]
    for child in topic:
        if (
            local_name(child.tag) == "pointer"
            and child.attrib.get("role") == "scenario_topic_type"
        ):
            return child.attrib.get("href", "topic").rsplit("#", 1)[-1]
    return topic.attrib.get(NITE_ID, "topic")


def contiguous_runs(values: Iterable[int]) -> list[tuple[int, int]]:
    ordered = sorted(set(values))
    if not ordered:
        return []
    runs = []
    start = previous = ordered[0]
    for value in ordered[1:]:
        if value != previous + 1:
            runs.append((start, previous))
            start = value
        previous = value
    runs.append((start, previous))
    return runs


def gaps(segment_ids: list[str], covered: set[int]) -> list[dict[str, str]]:
    return [
        {"start_segment_id": segment_ids[start], "end_segment_id": segment_ids[end]}
        for start, end in contiguous_runs(set(range(len(segment_ids))) - covered)
    ]


def convert_meeting(
    archive: zipfile.ZipFile, meeting: str
) -> tuple[dict[str, Any] | None, dict[str, int]]:
    counts = {
        "unmappedBoundaries": 0,
        "emptyTextUtterancesDropped": 0,
        "discontinuousTopicsSplit": 0,
        "deepTopicsFlattened": 0,
        "children": 0,
        "nestedTopics": 0,
    }
    words, ordered_ids = words_for_meeting(archive, meeting)
    utterances, unmapped_utterances, empty_text_utterances = utterances_for_meeting(
        archive, meeting, words, ordered_ids
    )
    counts["unmappedBoundaries"] += unmapped_utterances
    counts["emptyTextUtterancesDropped"] += empty_text_utterances
    if not utterances:
        return None, counts
    topic_root = parse_xml(archive, f"topics/{meeting}.topic.xml")
    top_topics = [
        element for element in topic_root if local_name(element.tag) == "topic"
    ]
    topic_sets = [topic_word_ids(topic, ordered_ids, True) for topic in top_topics]
    assignments: dict[int, int] = {}
    for utterance_index, utterance in enumerate(utterances):
        words_in_utterance = set(utterance["wordIds"])
        scored = [
            (len(words_in_utterance & topic_words), topic_index)
            for topic_index, topic_words in enumerate(topic_sets)
        ]
        count, topic_index = max(
            scored, default=(0, -1), key=lambda value: (value[0], -value[1])
        )
        if count:
            assignments[utterance_index] = topic_index
    segment_ids = [utterance["id"] for utterance in utterances]
    children = []
    for topic_index, topic in enumerate(top_topics):
        topic_runs = contiguous_runs(
            index for index, assigned in assignments.items() if assigned == topic_index
        )
        if len(topic_runs) > 1:
            counts["discontinuousTopicsSplit"] += 1
        for span_index, (start, end) in enumerate(topic_runs):
            children.append(
                {
                    "start_segment_id": segment_ids[start],
                    "end_segment_id": segment_ids[end],
                    "title": [topic_title(topic)],
                    "tags": [["ami-topic"]],
                    "summary": [],
                    "evidence_segment_ids": [segment_ids[start], segment_ids[end]]
                    if start != end
                    else [segment_ids[start]],
                    "sourceRef": {
                        "topicId": topic.attrib.get(NITE_ID),
                        "spanIndex": span_index,
                    },
                }
            )
    children.sort(key=lambda child: segment_ids.index(child["start_segment_id"]))
    nested_topics = []
    for parent_index, topic in enumerate(top_topics):
        for nested in [
            element for element in topic if local_name(element.tag) == "topic"
        ]:
            nested_words = topic_word_ids(nested, ordered_ids, True)
            matching = [
                index
                for index, utterance in enumerate(utterances)
                if assignments.get(index) == parent_index
                and set(utterance["wordIds"]) & nested_words
            ]
            for start, end in contiguous_runs(matching):
                nested_topics.append(
                    {
                        "depth": 2,
                        "parentTopicId": topic.attrib.get(NITE_ID),
                        "topicId": nested.attrib.get(NITE_ID),
                        "title": topic_title(nested),
                        "start_segment_id": segment_ids[start],
                        "end_segment_id": segment_ids[end],
                        "evidence_segment_ids": (
                            [segment_ids[start], segment_ids[end]]
                            if start != end
                            else [segment_ids[start]]
                        ),
                    }
                )
            counts["deepTopicsFlattened"] += sum(
                1
                for element in nested.iter()
                if element is not nested and local_name(element.tag) == "topic"
            )
    covered = set(assignments)
    uncovered = gaps(segment_ids, covered)
    counts["children"] = len(children)
    counts["nestedTopics"] = len(nested_topics)
    for utterance in utterances:
        utterance.pop("wordIds")
    record = {
        "schemaVersion": 1,
        "scenarioId": f"ami-{meeting}",
        "source": {"dataset": "AMI", "meetingId": meeting},
        "timing": "Original AMI annotation timing; simultaneous speakers may overlap.",
        "segments": utterances,
        "gold": {
            "allowed_mode": "partition" if not uncovered else "extract",
            "children": children,
            "uncovered_ranges": uncovered,
        },
        "topicHierarchy": {"maxTreeDepth": 2, "nestedTopics": nested_topics},
    }
    return record, counts


def convert(
    input_zip: Path,
    output: Path,
    limit: int | None = None,
    manifest_path: Path | None = None,
    license_path: Path | None = None,
) -> dict[str, Any]:
    output.parent.mkdir(parents=True, exist_ok=True)
    encoded = []
    report = {
        "schemaVersion": 1,
        "dataset": "AMI manual annotations",
        "license": "CC-BY-4.0",
        "inputMeetings": 0,
        "outputRecords": 0,
        "discardedMeetings": 0,
        "outputChildren": 0,
        "outputNestedTopics": 0,
        "discontinuousTopicsSplit": 0,
        "unmappedBoundaries": 0,
        "emptyTextUtterancesDropped": 0,
        "deepTopicsFlattened": 0,
        "maxTreeDepth": 2,
    }
    if manifest_path is not None:
        license_path = license_path or input_zip.with_name("LICENSE.html")
        dataset, evidence = verify_declared_files(
            manifest_path,
            "ami",
            {
                "ami/ami_public_manual_1.6.2.zip": input_zip,
                "ami/LICENSE.html": license_path,
            },
        )
        report["sourceVersion"] = dataset["version"]
        report["sourceFiles"] = evidence
        report["licenseEvidence"] = {
            "license": dataset["license"],
            "status": dataset["licenseStatus"],
            "snapshot": next(item for item in evidence if item["kind"] == "license"),
            "embeddedSnapshot": "LICENCE.txt",
        }
    with zipfile.ZipFile(input_zip) as archive:
        if "LICENCE.txt" not in archive.namelist():
            raise ValueError("AMI archive does not contain its LICENCE.txt snapshot")
        meetings = sorted(
            Path(name).name.removesuffix(".topic.xml")
            for name in archive.namelist()
            if name.startswith("topics/") and name.endswith(".topic.xml")
        )
        if not meetings:
            raise ValueError("AMI archive contains no manual topic annotations")
        if limit is not None:
            meetings = meetings[:limit]
        for meeting in meetings:
            report["inputMeetings"] += 1
            record, counts = convert_meeting(archive, meeting)
            for key in (
                "outputChildren",
                "outputNestedTopics",
                "discontinuousTopicsSplit",
                "unmappedBoundaries",
                "emptyTextUtterancesDropped",
                "deepTopicsFlattened",
            ):
                source_key = {
                    "outputChildren": "children",
                    "outputNestedTopics": "nestedTopics",
                }.get(key, key)
                report[key] += counts[source_key]
            if record is None:
                report["discardedMeetings"] += 1
                continue
            encoded.append(
                json.dumps(
                    record, ensure_ascii=False, sort_keys=True, separators=(",", ":")
                )
            )
            report["outputRecords"] += 1
    output_bytes = ("\n".join(encoded) + ("\n" if encoded else "")).encode("utf-8")
    output.write_bytes(output_bytes)
    report["outputBytes"] = len(output_bytes)
    report["outputSha256"] = hashlib.sha256(output_bytes).hexdigest()
    output.with_suffix(".report.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return report


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, default=DEFAULT_INPUT)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--license", type=Path)
    parser.add_argument(
        "--skip-source-verification",
        action="store_true",
        help="Allow converter unit fixtures that are not pinned public source files",
    )
    args = parser.parse_args()
    manifest_path = None if args.skip_source_verification else args.manifest
    print(
        json.dumps(
            convert(args.input, args.output, args.limit, manifest_path, args.license),
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
