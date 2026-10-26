#!/usr/bin/env python3
"""Score independent CPU DIA runs against audio-bound references, without ASR."""
import argparse
import hashlib
import json
import math
import os
import re
import statistics
import tempfile
import wave
from pathlib import Path

from check_diarization_cpu import load_run
from metrics import diarization

SCHEMA = "meetnote.diarization-quality.v1"


def evaluate_run(run, reference_path, collars):
    payload = reference_path.read_bytes()
    reference = json.loads(payload)
    report, rows = load_run(run, require_embeddings=False)
    if reference.get("schema") != "meetnote.diarization-reference.v1":
        raise ValueError("unsupported diarization reference schema")
    if report["configuration"].get("device") != "Cpu":
        raise ValueError("only independent CPU diarization runs are supported")
    backend = report["configuration"].get("backend")
    if backend not in ("diarization-native", "sherpa-onnx") or any(
        row.get("backend") != backend for row in rows
    ):
        raise ValueError("inconsistent backend provenance")
    duration = reference.get("durationMs")
    if type(duration) is not int or duration <= 0 or reference.get("sampleRateHz") != 16000:
        raise ValueError("invalid reference duration or sample rate")
    if any(type(reference.get(k)) is not int for k in ["firstSourceSample", "lastSourceSample"]):
        raise ValueError("missing source sample range")
    if (
        reference["firstSourceSample"] < 0
        or reference["lastSourceSample"] - reference["firstSourceSample"] != duration * 16
    ):
        raise ValueError("inconsistent source sample range")
    if reference.get("scoringRegionsMs") != [[0, duration]]:
        raise ValueError("unsupported scoring regions; full crop UEM coverage is required")
    if any(
        not reference.get(k)
        for k in [
            "meetingId",
            "dataset",
            "split",
            "referenceVariant",
            "referenceTiming",
            "sourceRepository",
            "sourceRevision",
        ]
    ):
        raise ValueError("missing reference provenance")
    seconds = report.get("audioSeconds")
    if (
        type(seconds) not in (int, float)
        or not math.isfinite(seconds)
        or seconds != duration / 1000
        or report["audioSha256"] != reference.get("audioSha256")
    ):
        raise ValueError("run audio hash or duration does not match reference")
    if not re.fullmatch(r"[0-9a-f]{64}", report.get("runnerSha256", "")):
        raise ValueError("missing native runner fingerprint")
    artifacts = report.get("modelArtifacts")
    if (
        not isinstance(artifacts, dict)
        or not artifacts
        or any(
            not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value)
            for value in artifacts.values()
        )
    ):
        raise ValueError("missing model artifact fingerprints")
    audio = Path(reference["audioPath"])
    with audio.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != reference["audioSha256"]:
            raise ValueError("reference audio checksum mismatch")
    with wave.open(str(audio), "rb") as stream:
        if (
            stream.getnchannels(),
            stream.getsampwidth(),
            stream.getframerate(),
            stream.getnframes(),
        ) != (1, 2, 16000, duration * 16):
            raise ValueError("reference audio PCM duration/format mismatch")
    if (
        not collars
        or any(type(c) is not int or c < 0 for c in collars)
        or len(set(collars)) != len(collars)
    ):
        raise ValueError("collars must be distinct nonnegative full durations in milliseconds")
    results = []
    for row in rows:
        if type(row["wallSeconds"]) not in (int, float):
            raise ValueError("invalid wall time")
        hypothesis = [
            {"speaker": s["speaker_id"], "startMs": s["start_ms"], "endMs": s["end_ms"]}
            for s in row["speakerSegments"]
        ]
        for collar in collars:
            score = diarization(reference["referenceSpeakerSegments"], hypothesis, collar, duration)
            results.append({"repeat": row["repeat"], "collarMs": collar, **score})
    return {
        "schema": SCHEMA,
        "status": "complete",
        "qualityAcceptance": "not_assessed",
        "scope": "independent CPU diarization diagnostic; no ASR or Assistant quality",
        "asrQualityStatus": "not_assessed",
        "assistantSemanticQualityStatus": "not_assessed",
        "meetingId": reference["meetingId"],
        "dataset": reference["dataset"],
        "split": reference["split"],
        "referenceVariant": reference["referenceVariant"],
        "referenceTiming": reference["referenceTiming"],
        "sourceRepository": reference["sourceRepository"],
        "sourceRevision": reference["sourceRevision"],
        "referenceSha256": hashlib.sha256(payload).hexdigest(),
        "inputReportSha256": report["loadedReportSha256"],
        "predictionSha256": report["predictionSha256"],
        "audioSha256": report["audioSha256"],
        "runnerSha256": report["runnerSha256"],
        "nativeSourceCommit": report.get("sourceCommit"),
        "scorerSha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "metricImplementationSha256": hashlib.sha256(
            Path(__file__).with_name("metrics.py").read_bytes()
        ).hexdigest(),
        "modelArtifacts": artifacts,
        "configuration": report["configuration"],
        "medianCallSeconds": statistics.median(row["wallSeconds"] for row in rows),
        "peakRssBytes": report["peakRssBytes"],
        "results": results,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--collar-ms", type=int, nargs="+", default=[0, 500])
    args = parser.parse_args()
    protected = [args.reference, args.run / "report.json", args.run / "prediction.jsonl"]
    if any(
        args.output.resolve() == path.resolve()
        or (args.output.exists() and path.exists() and args.output.samefile(path))
        for path in protected
    ):
        raise ValueError("output must not replace an input")
    if args.output.exists():
        # Only an earlier report of this tool is disposable, never an audio or
        # unrelated workspace asset merely passed as an output path.
        try:
            previous = json.loads(args.output.read_bytes())
        except (ValueError, UnicodeError) as error:
            raise ValueError("existing output is not a disposable diarization report") from error
        if not isinstance(previous, dict) or previous.get("schema") != SCHEMA:
            raise ValueError("existing output is not a disposable diarization report")
        args.output.unlink()
    result = evaluate_run(args.run, args.reference, args.collar_ms)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            dir=args.output.parent,
            prefix=".diarization-quality-",
            delete=False,
        ) as stream:
            temporary = Path(stream.name)
            json.dump(result, stream, indent=2)
            stream.write("\n")
        os.replace(temporary, args.output)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    print(
        json.dumps(
            {"output": str(args.output), "status": result["status"], "results": result["results"]}
        )
    )


if __name__ == "__main__":
    main()
