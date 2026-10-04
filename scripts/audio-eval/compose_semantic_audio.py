#!/usr/bin/env python3
"""Compose real dataset clips into a deterministic semantic-boundary case."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path


def reference_payload(duration_ms: int, segments: list[dict[str, object]]) -> dict[str, object]:
    return {
        "schemaVersion": 2,
        "type": "compositeAudioBoundaryReference",
        "evaluationKind": "audio_boundary_reference",
        "sourceDomain": "audio",
        "durationMs": duration_ms,
        "segments": segments,
        "assistantSemanticQualityStatus": "not_assessed",
        "assistantSemanticQualityReason": (
            "The composed audio is an Audio Host boundary fixture. It is not "
            "Assistant semantic gold data and must not be used to claim topic, "
            "title, summary, or document-tree quality."
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-audio", type=Path, required=True)
    parser.add_argument("--output-reference", type=Path, required=True)
    parser.add_argument("--clip", action="append", nargs=4, metavar=("PATH", "START_MS", "DURATION_MS", "TOPIC"), required=True)
    args = parser.parse_args()
    if len(args.clip) < 2:
        parser.error("at least two clips are required")
    args.output_audio.parent.mkdir(parents=True, exist_ok=True)
    args.output_reference.parent.mkdir(parents=True, exist_ok=True)
    filter_inputs: list[str] = []
    filter_parts: list[str] = []
    reference: list[dict[str, object]] = []
    cursor = 0
    for index, (path, start_ms, duration_ms, topic) in enumerate(args.clip):
        start = int(start_ms) / 1000
        duration = int(duration_ms) / 1000
        filter_inputs.extend(["-ss", str(start), "-t", str(duration), "-i", path])
        filter_parts.append(f"[{index}:a]aresample=16000,pan=mono|c0=c0[a{index}]")
        reference.append(
            {
                "startMs": cursor,
                "endMs": cursor + int(duration_ms),
                "topic": topic,
                "sourceAudio": path,
                "sourceStartMs": int(start_ms),
            }
        )
        cursor += int(duration_ms)
    labels = "".join(f"[a{index}]" for index in range(len(args.clip)))
    filter_complex = ";".join(filter_parts) + f";{labels}concat=n={len(args.clip)}:v=0:a=1[out]"
    command = [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        *filter_inputs,
        "-filter_complex",
        filter_complex,
        "-map",
        "[out]",
        "-c:a",
        "pcm_s16le",
        str(args.output_audio),
    ]
    subprocess.run(command, check=True)
    args.output_reference.write_text(
        json.dumps(
            reference_payload(cursor, reference),
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"durationMs": cursor, "clips": len(reference)}))


if __name__ == "__main__":
    main()
