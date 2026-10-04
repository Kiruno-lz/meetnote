#!/usr/bin/env python3
"""Convert CHiME-6 JSON annotations and array audio to canonical JSONL."""

from __future__ import annotations

import argparse
import json
import re
from collections import defaultdict
from pathlib import Path


TIME = re.compile(r"^(?P<h>\d+):(?P<m>\d+):(?P<s>\d+(?:\.\d+)?)$")


def milliseconds(value: object) -> int:
    if isinstance(value, dict):
        value = next(
            (value[key] for key in sorted(value) if key.startswith("U")),
            value.get("original"),
        )
    match = TIME.match(str(value))
    if not match:
        raise ValueError(f"invalid CHiME-6 timestamp: {value!r}")
    return round(
        (
            int(match.group("h")) * 3600
            + int(match.group("m")) * 60
            + float(match.group("s"))
        )
        * 1000
    )


def locate(root: Path, *parts: str) -> Path:
    candidates = [root.joinpath(*parts)]
    candidates.extend(root.glob(f"*/{'/'.join(parts)}"))
    candidates.extend(
        path
        for path in root.rglob(parts[-1])
        if path.is_dir() and tuple(path.parts[-len(parts) :]) == parts
    )
    for candidate in candidates:
        if candidate.is_dir():
            return candidate
    return candidates[0]


def convert(
    audio_root_arg: Path, transcription_root_arg: Path, split: str, output: Path
) -> None:
    transcription_root = locate(transcription_root_arg, split)
    audio_root = locate(audio_root_arg, "audio", split)
    if not transcription_root.is_dir():
        raise ValueError(f"missing CHiME-6 transcription directory: {transcription_root}")
    json_files = sorted(transcription_root.rglob("*.json"))
    records = []
    for annotation in json_files:
        payload = json.loads(annotation.read_text(encoding="utf-8"))
        session = (
            (payload.get("session_id") or annotation.stem)
            if isinstance(payload, dict)
            else annotation.stem
        )
        utterances = (
            payload
            if isinstance(payload, list)
            else payload.get("utterances", payload.get("transcriptions", []))
        )
        if isinstance(utterances, dict):
            utterances = utterances.get("utterances", [])
        if not utterances:
            continue
        grouped: dict[str, list[dict[str, object]]] = defaultdict(list)
        for utterance in utterances:
            ref = str(utterance.get("ref", "")) if isinstance(utterance, dict) else ""
            grouped[ref].append(utterance)
        for ref, ref_utterances in sorted(grouped.items()):
            if not ref:
                ref = "full"
            recording = session if ref == "full" else f"{session}_{ref}"
            array_files = sorted(audio_root.rglob(f"{recording}.CH*.wav"))
            if not array_files:
                raise ValueError(f"missing CHiME-6 array audio for {recording}")
            reference = []
            for utterance in ref_utterances:
                start = milliseconds(utterance["start_time"])
                end = milliseconds(utterance["end_time"])
                if end <= start or "speaker" not in utterance:
                    continue
                reference.append(
                    {
                        "startMs": start,
                        "endMs": end,
                        "speaker": utterance["speaker"],
                        "text": utterance.get("words", ""),
                    }
                )
            if not reference:
                continue
            records.append(
                {
                    "schemaVersion": 1,
                    "dataset": "chime6",
                    "split": split,
                    "meetingId": recording,
                    "audioPath": str(array_files[0]),
                    "language": "en",
                    "speakerCount": len({item["speaker"] for item in reference}),
                    "transcript": reference,
                    "referenceSpeakerSegments": [
                        {key: item[key] for key in ("startMs", "endMs", "speaker")}
                        for item in reference
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
    parser.add_argument("audio_root", type=Path)
    parser.add_argument("transcription_root", type=Path)
    parser.add_argument("split", choices=("dev", "eval"))
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    convert(args.audio_root, args.transcription_root, args.split, args.output)


if __name__ == "__main__":
    main()
