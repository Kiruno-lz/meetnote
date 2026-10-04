#!/usr/bin/env python3
"""Convert AliMeeting TextGrid references to a canonical JSONL projection."""

from __future__ import annotations

import argparse
import ast
import json
import re
from collections import defaultdict
from pathlib import Path


INTERVAL = re.compile(
    r"intervals \[\d+\]:\s+xmin = ([0-9.]+)\s+xmax = ([0-9.]+)\s+text = (.+)",
    re.MULTILINE,
)
SPEAKER = re.compile(r"(?:^|_)SPK(?P<speaker>[^_]+)$")


def parse_textgrid(path: Path) -> tuple[str, list[dict[str, object]]]:
    meeting = path.stem
    segments = []
    content = path.read_text(encoding="utf-8")
    tiers = re.split(r"(?=\titem \[\d+\]:)", content)
    for tier in tiers:
        name = re.search(r"\n\t\tname = \"([^\"]+)\"", tier)
        if not name:
            continue
        match = SPEAKER.search(name.group(1))
        if not match:
            continue
        speaker = match.group("speaker")
        for start, end, text in INTERVAL.findall(tier):
            value = ast.literal_eval(text.strip())
            if not value:
                continue
            segments.append(
                {
                    "speaker": speaker,
                    "startMs": round(float(start) * 1000),
                    "endMs": round(float(end) * 1000),
                    "text": value,
                }
            )
    return meeting, segments


def convert(root: Path, output: Path) -> None:
    prefix = root.name
    far = root / f"{prefix}_far"
    if not far.is_dir():
        candidates = sorted(root.rglob("*_far"))
        if len(candidates) != 1:
            raise ValueError(
                f"expected one AliMeeting far-field directory under {root}, got {candidates}"
            )
        far = candidates[0]
        prefix = far.name.removesuffix("_far")
    split = prefix.removesuffix("_Ali").lower()
    textgrid_dir = far / "textgrid_dir"
    audio_dir = far / "audio_dir"
    meetings: dict[str, list[dict[str, object]]] = defaultdict(list)
    for path in sorted(textgrid_dir.glob("*.TextGrid")):
        meeting, segments = parse_textgrid(path)
        meetings[meeting].extend(segments)

    records = []
    for meeting, segments in sorted(meetings.items()):
        audio_candidates = sorted(audio_dir.glob(f"{meeting}_MS*.wav"))
        if len(audio_candidates) != 1:
            raise ValueError(
                f"expected one far-field audio for {meeting}, got {audio_candidates}"
            )
        audio = audio_candidates[0]
        segments.sort(key=lambda item: (int(item["startMs"]), str(item["speaker"])))
        records.append(
            {
                "schemaVersion": 1,
                "dataset": "alimeeting",
                "split": split,
                "meetingId": meeting,
                "audioPath": str(audio),
                "language": "zh",
                "speakerCount": len({item["speaker"] for item in segments}),
                "transcript": [
                    {
                        "startMs": item["startMs"],
                        "endMs": item["endMs"],
                        "speaker": item["speaker"],
                        "text": item["text"],
                    }
                    for item in segments
                ],
                "referenceSpeakerSegments": [
                    {
                        "startMs": item["startMs"],
                        "endMs": item["endMs"],
                        "speaker": item["speaker"],
                    }
                    for item in segments
                ],
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
