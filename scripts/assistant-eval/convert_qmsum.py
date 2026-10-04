#!/usr/bin/env python3
"""Convert pinned QMSum JSONL into stable MeetNote text-level examples."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

from fetch import DEFAULT_MANIFEST, verify_declared_files


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_INPUT = ROOT / "eval-data/assistant/data/qmsum"
DEFAULT_OUTPUT = ROOT / "eval-data/assistant/data/converted/qmsum.jsonl"


def canonical(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def stable_segment_id(split: str, row_index: int, turn_index: int) -> str:
    return f"qmsum-{split}-{row_index:04d}-seg-{turn_index:05d}"


def uncovered_ranges(segment_ids: list[str], covered: set[int]) -> list[dict[str, str]]:
    ranges = []
    cursor = 0
    while cursor < len(segment_ids):
        if cursor in covered:
            cursor += 1
            continue
        start = cursor
        while cursor + 1 < len(segment_ids) and cursor + 1 not in covered:
            cursor += 1
        ranges.append(
            {
                "start_segment_id": segment_ids[start],
                "end_segment_id": segment_ids[cursor],
            }
        )
        cursor += 1
    return ranges


def convert_topic(
    source: dict[str, Any], split: str, row_index: int, topic_index: int
) -> tuple[dict[str, Any] | None, dict[str, int]]:
    """Create one extract example per topic so distinct QMSum topics never overlap."""
    scenario_id = f"qmsum-{split}-{row_index:04d}-topic-{topic_index:03d}"
    segments = []
    for turn_index, turn in enumerate(source["meeting_transcripts"]):
        segments.append(
            {
                "id": stable_segment_id(split, row_index, turn_index),
                "startMs": turn_index * 1_000,
                "endMs": (turn_index + 1) * 1_000,
                "speaker": turn.get("speaker") or None,
                "text": turn["content"],
                "sourceRef": {"turnIndex": turn_index},
            }
        )
    segment_ids = [segment["id"] for segment in segments]
    topic = source["topic_list"][topic_index]
    children = []
    covered: set[int] = set()
    unmapped = 0
    raw_spans = topic.get("relevant_text_span", [])
    for span_index, span in enumerate(raw_spans):
        try:
            start, end = int(span[0]), int(span[1])
        except (IndexError, TypeError, ValueError):
            unmapped += 1
            continue
        if start < 0 or end < start or end >= len(segment_ids):
            unmapped += 1
            continue
        covered.update(range(start, end + 1))
        children.append(
            {
                "start_segment_id": segment_ids[start],
                "end_segment_id": segment_ids[end],
                "title": [topic.get("topic") or f"Topic {topic_index + 1}"],
                "tags": [["qmsum-topic"]],
                "summary": [],
                "evidence_segment_ids": [segment_ids[start], segment_ids[end]]
                if start != end
                else [segment_ids[start]],
                "sourceRef": {
                    "topicIndex": topic_index,
                    "spanIndex": span_index,
                    "turnSpan": [start, end],
                },
            }
        )
    children.sort(key=lambda child: segment_ids.index(child["start_segment_id"]))
    if not children:
        return None, {
            "discontinuousTopicsSplit": int(len(raw_spans) > 1),
            "unmappedSpans": unmapped,
            "children": 0,
        }
    gaps = uncovered_ranges(segment_ids, covered)
    record = {
        "schemaVersion": 1,
        "scenarioId": scenario_id,
        "source": {
            "dataset": "QMSum",
            "split": split,
            "rowIndex": row_index,
            "topicIndex": topic_index,
        },
        "timing": "Synthetic monotonic text-level timing; not an audio-boundary measurement.",
        "segments": segments,
        "gold": {
            "allowed_mode": "partition"
            if not gaps and len(covered) == len(segment_ids)
            else "extract",
            "children": children,
            "uncovered_ranges": gaps,
        },
        "queries": source.get("specific_query_list", []),
    }
    return record, {
        "discontinuousTopicsSplit": int(len(raw_spans) > 1),
        "unmappedSpans": unmapped,
        "children": len(children),
    }


def convert(
    input_dir: Path,
    output: Path,
    limit: int | None = None,
    manifest_path: Path | None = None,
) -> dict[str, Any]:
    output.parent.mkdir(parents=True, exist_ok=True)
    encoded_records: list[str] = []
    report = {
        "schemaVersion": 1,
        "dataset": "QMSum",
        "license": "MIT",
        "inputMeetings": 0,
        "inputTopics": 0,
        "outputRecords": 0,
        "discardedMeetings": 0,
        "discardedTopics": 0,
        "outputChildren": 0,
        "discontinuousTopicsSplit": 0,
        "unmappedSpans": 0,
        "syntheticTiming": True,
    }
    if manifest_path is not None:
        paths = {
            f"qmsum/{split}.jsonl": input_dir / f"{split}.jsonl"
            for split in ("train", "val", "test")
        }
        paths["qmsum/LICENSE"] = input_dir / "LICENSE"
        dataset, evidence = verify_declared_files(manifest_path, "qmsum", paths)
        report["sourceVersion"] = dataset["version"]
        report["sourceFiles"] = evidence
        report["licenseEvidence"] = {
            "license": dataset["license"],
            "status": dataset["licenseStatus"],
            "snapshot": next(item for item in evidence if item["kind"] == "license"),
        }
    seen_split = False
    for split in ("train", "val", "test"):
        path = input_dir / f"{split}.jsonl"
        if not path.exists():
            continue
        seen_split = True
        with path.open(encoding="utf-8") as handle:
            for row_index, line in enumerate(handle):
                if limit is not None and report["outputRecords"] >= limit:
                    break
                report["inputMeetings"] += 1
                try:
                    source = json.loads(line)
                    topics = source["topic_list"]
                except (json.JSONDecodeError, KeyError, TypeError, ValueError):
                    report["discardedMeetings"] += 1
                    continue
                before = report["outputRecords"]
                for topic_index in range(len(topics)):
                    if limit is not None and report["outputRecords"] >= limit:
                        break
                    report["inputTopics"] += 1
                    try:
                        converted, counts = convert_topic(
                            source, split, row_index, topic_index
                        )
                    except (KeyError, TypeError, ValueError):
                        report["discardedTopics"] += 1
                        continue
                    report["outputChildren"] += counts["children"]
                    report["discontinuousTopicsSplit"] += counts[
                        "discontinuousTopicsSplit"
                    ]
                    report["unmappedSpans"] += counts["unmappedSpans"]
                    if converted is None:
                        report["discardedTopics"] += 1
                        continue
                    encoded_records.append(canonical(converted))
                    report["outputRecords"] += 1
                if report["outputRecords"] == before:
                    report["discardedMeetings"] += 1
            if limit is not None and report["outputRecords"] >= limit:
                break
    if not seen_split:
        raise FileNotFoundError(f"no QMSum split files found under {input_dir}")
    output_bytes = (
        "\n".join(encoded_records) + ("\n" if encoded_records else "")
    ).encode("utf-8")
    output.write_bytes(output_bytes)
    report["outputBytes"] = len(output_bytes)
    report["outputSha256"] = hashlib.sha256(output_bytes).hexdigest()
    report_path = output.with_suffix(".report.json")
    report_path.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return report


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input-dir", type=Path, default=DEFAULT_INPUT)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument(
        "--skip-source-verification",
        action="store_true",
        help="Allow converter unit fixtures that are not pinned public source files",
    )
    args = parser.parse_args()
    manifest_path = None if args.skip_source_verification else args.manifest
    print(
        json.dumps(
            convert(args.input_dir, args.output, args.limit, manifest_path),
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
