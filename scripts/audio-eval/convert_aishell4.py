#!/usr/bin/env python3
"""Convert AISHELL-4 RTTM/TextGrid references to canonical JSONL."""

from __future__ import annotations

import argparse
import ast
import json
import re
from pathlib import Path


INTERVAL = re.compile(
    r"intervals \[\d+\]:\s+xmin = ([0-9.]+)\s+xmax = ([0-9.]+)\s+text = (.+)",
    re.MULTILINE,
)


def textgrid_segments(path: Path) -> list[dict[str, object]]:
    content = path.read_text(encoding="utf-8")
    segments: list[dict[str, object]] = []
    for tier in re.split(r"(?=\s*item \[\d+\]:)", content):
        name = re.search(r"\n\s+name = \"([^\"]+)\"", tier)
        if not name:
            continue
        for start, end, text in INTERVAL.findall(tier):
            value = ast.literal_eval(text.strip()).replace("<sil>", "").strip()
            if not value:
                continue
            segments.append(
                {
                    "startMs": round(float(start) * 1000),
                    "endMs": round(float(end) * 1000),
                    "speaker": name.group(1),
                    "text": value,
                }
            )
    return sorted(segments, key=lambda item: (item["startMs"], item["speaker"]))


def rttm_segments(path: Path) -> list[dict[str, object]]:
    segments = []
    for line in path.read_text(encoding="utf-8").splitlines():
        fields = line.split()
        if not fields or fields[0] != "SPEAKER" or len(fields) < 8:
            continue
        segments.append(
            {
                "startMs": round(float(fields[3]) * 1000),
                "endMs": round((float(fields[3]) + float(fields[4])) * 1000),
                "speaker": fields[7],
            }
        )
    return sorted(segments, key=lambda item: (item["startMs"], item["speaker"]))


def convert(root: Path, output: Path) -> None:
    base = root / "test"
    audio_dir = base / "wav"
    annotation_dir = base / "TextGrid"
    records = []
    for rttm in sorted(annotation_dir.glob("*.rttm")):
        record_id = rttm.stem
        audio = audio_dir / f"{record_id}.flac"
        textgrid = annotation_dir / f"{record_id}.TextGrid"
        if not audio.is_file() or not textgrid.is_file():
            raise ValueError(f"incomplete AISHELL-4 reference for {record_id}")
        speaker_segments = rttm_segments(rttm)
        transcript = textgrid_segments(textgrid)
        records.append(
            {
                "schemaVersion": 1,
                "dataset": "aishell4",
                "split": "test",
                "meetingId": record_id,
                "audioPath": str(audio),
                "language": "zh",
                "speakerCount": len({item["speaker"] for item in speaker_segments}),
                "transcript": transcript,
                "referenceSpeakerSegments": speaker_segments,
            }
        )
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        "".join(json.dumps(record, ensure_ascii=False) + "\n" for record in records),
        encoding="utf-8",
    )
    print(json.dumps({"records": len(records), "output": str(output)}, ensure_ascii=False))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    convert(args.root, args.output)


if __name__ == "__main__":
    main()
