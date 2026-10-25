#!/usr/bin/env python3
"""Build pinned native FLEURS B1 cases and separately labelled stress variants."""
from __future__ import annotations
import argparse
import array
import csv
import hashlib
import io
import json
import math
import random
import subprocess
import tarfile
import urllib.request
import wave
from pathlib import Path

REVISION = "70bb2e84b976b7e960aa89f1c648e09c59f894dd"
SOURCES = {
    "zh": ("cmn_hans_cn", "09d19ad18f5d7e91076880807e866cd16abd924c7052b55f71cdae91714fc166"),
    "en": ("en_us", "d9c2e37b41aacd41bc283554a0a82b5476b36887049774ecb2819dcaaa55a356"),
}


def sha256(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def write_wave(path, samples):
    with wave.open(str(path), "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(16000)
        out.writeframes(samples)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--count", type=int, default=24)
    parser.add_argument(
        "--offset", type=int, default=0, help="skip this many distinct sentence IDs"
    )
    parser.add_argument("--language", action="append", choices=("zh", "en"))
    args = parser.parse_args()
    if args.count < 1 or args.offset < 0:
        parser.error("count must be positive and offset nonnegative")
    root = args.root.resolve()
    (root / "audio").mkdir(parents=True, exist_ok=True)
    cases = []
    sources = []
    for language, (config, expected) in SOURCES.items():
        if args.language and language not in args.language:
            continue
        archive = root / "downloads" / (language + "-test.tar.gz")
        if sha256(archive) != expected:
            raise ValueError(f"archive checksum mismatch: {archive}")
        url = f"https://huggingface.co/datasets/google/fleurs/resolve/{REVISION}/data/{config}/test.tsv"
        tsv = urllib.request.urlopen(url, timeout=60).read()
        (root / "downloads" / (language + "-test.tsv")).write_bytes(tsv)
        rows = []
        seen = set()
        for row in csv.reader(io.StringIO(tsv.decode()), delimiter="\t"):
            if row[0] not in seen:
                seen.add(row[0])
                if len(seen) > args.offset:
                    rows.append(row)
            if len(rows) == args.count:
                break
        if len(rows) != args.count:
            raise ValueError("insufficient unique reference IDs")
        sources.append(
            {
                "language": language,
                "config": config,
                "revision": REVISION,
                "archiveSha256": expected,
                "tsvSha256": hashlib.sha256(tsv).hexdigest(),
                "license": "CC-BY-4.0",
                "split": "test",
                "selection": "distinct sentence IDs in official TSV order",
                "offset": args.offset,
            }
        )
        selected = {row[1]: row for row in rows}
        combined = bytearray()
        references = []
        members = {}
        with tarfile.open(archive, "r:gz") as tar:
            for member in tar:
                basename = Path(member.name).name
                if basename in selected and member.isfile():
                    # Read only selected members; never extract archive paths to disk.
                    raw = tar.extractfile(member).read()
                    row = selected[basename]
                    path = root / "audio" / f"{language}-{row[0]}.wav"
                    subprocess.run(
                        [
                            "ffmpeg",
                            "-v",
                            "error",
                            "-y",
                            "-i",
                            "pipe:0",
                            "-ar",
                            "16000",
                            "-ac",
                            "1",
                            "-c:a",
                            "pcm_s16le",
                            str(path),
                        ],
                        input=raw,
                        check=True,
                    )
                    with wave.open(str(path), "rb") as audio:
                        frames = audio.readframes(audio.getnframes())
                    members[basename] = (row, path, frames)
        for row in rows:
            row, path, frames = members[row[1]]
            cases.append(
                {
                    "id": f"fleurs-{language}-{row[0]}",
                    "audioPath": str(path),
                    "sha256": sha256(path),
                    "reference": row[3],
                    "language": language,
                    "durationSeconds": len(frames) / 32000,
                    "scenario": "native",
                    "sourceFile": row[1],
                    "gender": row[6],
                }
            )
            combined.extend(frames)
            combined.extend(b"\0" * 12800)
            references.append(row[3])
        for scenario in ("composed-clean", "composed-noise20db"):
            data = bytes(combined)
            if scenario == "composed-noise20db":
                samples = array.array("h")
                samples.frombytes(data)
                rms = math.sqrt(sum(float(s) * s for s in samples) / len(samples))
                rng = random.Random(20261025)
                samples = array.array(
                    "h",
                    (max(-32768, min(32767, round(s + rng.gauss(0, rms / 10)))) for s in samples),
                )
                data = samples.tobytes()
            path = root / "audio" / f"{language}-{scenario}.wav"
            write_wave(path, data)
            cases.append(
                {
                    "id": f"fleurs-{language}-{scenario}",
                    "audioPath": str(path),
                    "sha256": sha256(path),
                    "reference": " ".join(references),
                    "language": language,
                    "durationSeconds": len(data) / 32000,
                    "scenario": scenario,
                    "construction": {
                        "sourceIds": [r[0] for r in rows],
                        "silenceSeconds": 0.4,
                        "noiseSeed": 20261025 if "noise" in scenario else None,
                    },
                }
            )
    (root / "cases.json").write_text(json.dumps(cases, ensure_ascii=False, indent=2) + "\n")
    (root / "dataset-receipt.json").write_text(
        json.dumps(
            {
                "schema": "meetnote.fleurs-b1.v1",
                "sources": sources,
                "ffmpeg": subprocess.check_output(["ffmpeg", "-version"]).decode().splitlines()[0],
                "nativeCases": len(sources) * args.count,
                "constructedCases": 2 * len(sources),
                "casesSha256": sha256(root / "cases.json"),
            },
            indent=2,
        )
        + "\n"
    )
    print(
        json.dumps(
            {"cases": len(cases), "seconds": sum(c["durationSeconds"] for c in cases)}, indent=2
        )
    )


if __name__ == "__main__":
    main()
