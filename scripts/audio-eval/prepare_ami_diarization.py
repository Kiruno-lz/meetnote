#!/usr/bin/env python3
"""Project the pinned AMI only-words train reference onto a verified PCM crop."""
import argparse
import hashlib
import io
import json
import os
import tempfile
import wave
from pathlib import Path

from prepare_ami_asr import AUDIO_SHA, sha256

MEETING = "ES2002a"
REVISION = "67c2d539286e89f68952d5dcf83912bd9f01dfae"
RTTM_SHA = "3153efc675b16b7c47c6e49a3b48602613b3dfcf0ca16f590fb7541176c0f0b2"
UEM_SHA = "684314dfa676ae080a891f75c9f1efaaf7df4905674d29939b758ada92c491f7"
SPLIT_SHA = "752c2a2b5af083c3fc468d524269b6481a4610572b81df44026a0a94b05e2f56"


def verify_crop(source, crop, first):
    if type(first) is not int or first < 0:
        raise ValueError("invalid crop sample offset")
    with wave.open(str(source), "rb") as original, wave.open(str(crop), "rb") as selected:
        for stream in [original, selected]:
            if (stream.getnchannels(), stream.getsampwidth(), stream.getframerate()) != (
                1,
                2,
                16000,
            ):
                raise ValueError("reference crops require 16 kHz mono signed 16-bit PCM")
        remaining = selected.getnframes()
        last = first + remaining
        if remaining == 0 or last > original.getnframes():
            raise ValueError("crop exceeds source PCM range")
        original.setpos(first)
        while remaining:
            count = min(16000, remaining)
            expected, actual = original.readframes(count), selected.readframes(count)
            if len(actual) != count * 2 or actual != expected:
                raise ValueError("crop PCM does not match the declared source offset")
            remaining -= count
    return last


def project_speakers(reference, uem, first, last):
    from pyannote.core import Segment

    if any(type(v) is not int for v in (first, last)) or not 0 <= first < last:
        raise ValueError("invalid crop sample range")
    if first % 16 or last % 16:
        raise ValueError("reference crop must use whole milliseconds")
    span = Segment(first / 16000, last / 16000)
    covered = list(uem.crop(span, mode="intersection").support())
    if covered != [span]:
        raise ValueError("crop is not fully covered by the pinned UEM")
    result = []
    for segment, _, speaker in reference.crop(span, mode="intersection").itertracks(
        yield_label=True
    ):
        start = round((segment.start - span.start) * 1000)
        end = round((segment.end - span.start) * 1000)
        if (
            not 0 <= start < end <= (last - first) // 16
            or not isinstance(speaker, str)
            or not speaker
        ):
            raise ValueError("reference speaker interval cannot be projected to valid milliseconds")
        result.append({"startMs": start, "endMs": end, "speaker": speaker})
    return sorted(result, key=lambda row: (row["startMs"], row["endMs"], row["speaker"]))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-audio", type=Path, required=True)
    parser.add_argument("--audio", type=Path, required=True)
    parser.add_argument("--rttm", type=Path, required=True)
    parser.add_argument("--uem", type=Path, required=True)
    parser.add_argument("--split-file", type=Path, required=True)
    parser.add_argument("--start-ms", type=int, default=0)
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    output = args.root / "diarization-reference.json"
    inputs = [args.source_audio, args.audio, args.rttm, args.uem, args.split_file]
    if any(
        output.resolve() == p.resolve() or (output.exists() and p.exists() and output.samefile(p))
        for p in inputs
    ):
        raise ValueError("reference output must not overwrite an input")
    if args.start_ms < 0:
        raise ValueError("start-ms must be nonnegative")
    if sha256(args.source_audio) != AUDIO_SHA:
        raise ValueError("source audio checksum mismatch")
    snapshots = {}
    for name, path, expected in [
        ("rttm", args.rttm, RTTM_SHA),
        ("uem", args.uem, UEM_SHA),
        ("split", args.split_file, SPLIT_SHA),
    ]:
        content = path.read_bytes()
        if hashlib.sha256(content).hexdigest() != expected:
            raise ValueError(f"{name} checksum mismatch")
        snapshots[name] = content
    if MEETING not in snapshots["split"].decode().splitlines():
        raise ValueError("meeting is not in the pinned training partition")
    audio_sha = sha256(args.audio)
    first = args.start_ms * 16
    last = verify_crop(args.source_audio, args.audio, first)
    from pyannote.database.util import load_rttm, load_uem

    references = load_rttm(io.StringIO(snapshots["rttm"].decode()))
    regions = load_uem(io.StringIO(snapshots["uem"].decode()))
    if set(references) != {MEETING} or set(regions) != {MEETING}:
        raise ValueError("reference recording identity mismatch")
    projected = project_speakers(references[MEETING], regions[MEETING], first, last)
    if sha256(args.source_audio) != AUDIO_SHA or sha256(args.audio) != audio_sha:
        raise ValueError("audio changed during reference projection")
    reference = {
        "schema": "meetnote.diarization-reference.v1",
        "meetingId": MEETING,
        "dataset": "AMI",
        "split": "train",
        "referenceVariant": "only_words",
        "referenceTiming": "word-activity intervals derived from AMI manual-1.6.2; forced-alignment times, not manually verified acoustic boundaries",
        "sourceRepository": "https://github.com/pyannote/AMI-diarization-setup",
        "sourceRevision": REVISION,
        "sourceRttmPath": "only_words/rttms/train/ES2002a.rttm",
        "sourceUemPath": "uems/train/ES2002a.uem",
        "sourceSplitPath": "lists/train.meetings.txt",
        "audioVariant": "Mix-Headset",
        "sourceAudioSha256": AUDIO_SHA,
        "sourceRttmSha256": RTTM_SHA,
        "sourceUemSha256": UEM_SHA,
        "splitSha256": SPLIT_SHA,
        "sampleRateHz": 16000,
        "firstSourceSample": first,
        "lastSourceSample": last,
        "audioPath": str(args.audio.resolve()),
        "audioSha256": audio_sha,
        "durationMs": (last - first) // 16,
        "scoringRegionsMs": [[0, (last - first) // 16]],
        "referenceSpeakerSegments": projected,
        "qualityAcceptance": "not_assessed",
    }
    args.root.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=args.root, prefix=".ami-reference-", delete=False
        ) as stream:
            temporary = Path(stream.name)
            json.dump(reference, stream, indent=2, ensure_ascii=False)
            stream.write("\n")
        os.replace(temporary, output)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    print(
        json.dumps(
            {
                "reference": str(output),
                "speakerSegments": len(projected),
                "durationMs": reference["durationMs"],
                "split": "train",
            }
        )
    )


if __name__ == "__main__":
    main()
