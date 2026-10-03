#!/usr/bin/env python3
"""Diagnose AMI input integrity versus topic-segmentation quality.

AMI evaluation uses manual transcript and speaker annotations.  This report
must not be interpreted as ASR or diarization quality evidence.  It checks the
converted text/speaker/timing layer, then compares speaker-change boundaries
with manual topic boundaries to detect whether diarization could explain a
topic-boundary failure.
"""

from __future__ import annotations

import argparse
import json
import statistics
from pathlib import Path
from typing import Any

from evaluate import boundary_prf, labels_from_children, read_jsonl


def diagnose(path: Path) -> dict[str, Any]:
    records = read_jsonl(path)
    empty_text = 0
    invalid_timing = 0
    duplicate_ids = 0
    missing_speaker = 0
    overlap_count = 0
    gap_count = 0
    speaker_f1: list[float] = []
    speaker_precision: list[float] = []
    speaker_recall: list[float] = []

    for record in records:
        segments = record["segments"]
        ids = [segment["id"] for segment in segments]
        duplicate_ids += int(len(ids) != len(set(ids)))
        gold_labels = labels_from_children(ids, record["gold"]["children"])
        gold_boundaries = sorted(
            index for index in range(1, len(gold_labels)) if gold_labels[index] != gold_labels[index - 1]
        )
        speaker_boundaries = [
            index
            for index in range(1, len(segments))
            if segments[index]["speaker"] != segments[index - 1]["speaker"]
        ]
        precision, recall, f1 = boundary_prf(gold_boundaries, speaker_boundaries)
        speaker_precision.append(precision)
        speaker_recall.append(recall)
        speaker_f1.append(f1)
        for left, right in zip(segments, segments[1:]):
            if left["endMs"] > right["startMs"]:
                overlap_count += 1
            if left["endMs"] < right["startMs"]:
                gap_count += 1
        for segment in segments:
            empty_text += int(not segment.get("text", "").strip())
            invalid_timing += int(
                not isinstance(segment.get("startMs"), int)
                or not isinstance(segment.get("endMs"), int)
                or segment["endMs"] <= segment["startMs"]
            )
            missing_speaker += int(not isinstance(segment.get("speaker"), str) or not segment["speaker"])

    segment_count = sum(len(record["segments"]) for record in records)
    adjacent_pairs = max(0, segment_count - len(records))
    input_integrity = {
        "emptyTextSegments": empty_text,
        "invalidTimingSegments": invalid_timing,
        "duplicateIdRecords": duplicate_ids,
        "missingSpeakerSegments": missing_speaker,
        "status": "passed"
        if not any((empty_text, invalid_timing, duplicate_ids, missing_speaker))
        else "failed",
    }
    return {
        "schemaVersion": 1,
        "dataset": "AMI manual annotations",
        "sourceFile": str(path),
        "recordCount": len(records),
        "segmentCount": segment_count,
        "inputIntegrity": input_integrity,
        "timing": {
            "overlappingAdjacentPairs": overlap_count,
            "gappedAdjacentPairs": gap_count,
            "adjacentPairCount": adjacent_pairs,
            "overlapIsExpectedFor": "AMI simultaneous speakers; not a diarization error by itself",
        },
        "speakerChangeVsTopicBoundary": {
            "boundaryPrecision": statistics.mean(speaker_precision) if speaker_precision else 0.0,
            "boundaryRecall": statistics.mean(speaker_recall) if speaker_recall else 0.0,
            "boundaryF1": statistics.mean(speaker_f1) if speaker_f1 else 0.0,
            "interpretation": "speaker changes do not explain manual topic boundaries",
        },
        "asrQualityStatus": "not_assessed",
        "diarizationQualityStatus": "not_assessed",
        "topicSegmentationDiagnosis": "assistant_topic_segmentation_quality_gap",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = diagnose(args.input)
    encoded = json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded, encoding="utf-8")
    print(encoded, end="")


if __name__ == "__main__":
    main()
