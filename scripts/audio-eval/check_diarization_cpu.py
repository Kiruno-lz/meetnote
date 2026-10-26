#!/usr/bin/env python3
"""Gate CPU embedding thread changes against identical inputs and full DIA output."""
import argparse
import hashlib
import json
import math
import statistics
from pathlib import Path


def load_run(path):
    report = json.loads((path / "report.json").read_text())
    content = (path / "prediction.jsonl").read_bytes()
    if report["schema"] != "meetnote.diarization-cpu-run.v1" or report["failure"]:
        raise ValueError("failed or unsupported run")
    if hashlib.sha256(content).hexdigest() != report["predictionSha256"]:
        raise ValueError("prediction checksum mismatch")
    rows = [json.loads(line) for line in content.splitlines()]
    repeats = report["configuration"]["repeats"]
    if (
        type(repeats) is not int
        or repeats < 2
        or any(type(row["repeat"]) is not int for row in rows)
        or len(rows) != repeats
        or {r["repeat"] for r in rows} != set(range(repeats))
    ):
        raise ValueError("incomplete or duplicate repeat coverage")
    for row in rows:
        if (
            row["status"] != "ok"
            or not math.isfinite(row["wallSeconds"])
            or row["wallSeconds"] <= 0
        ):
            raise ValueError("failed or invalid measurement")
        if (
            not row["speakerSegments"]
            or not row["embeddingObservations"]
            or row["segments"] != len(row["speakerSegments"])
            or row["observations"] != len(row["embeddingObservations"])
        ):
            raise ValueError("missing speaker timeline or embeddings")
        for seg in row["speakerSegments"]:
            if not 0 <= seg["start_ms"] < seg["end_ms"] <= report["audioSeconds"] * 1000:
                raise ValueError("invalid speaker timeline")
    if type(report["peakRssBytes"]) is not int or report["peakRssBytes"] <= 0:
        raise ValueError("missing process peak RSS")
    return report, sorted(rows, key=lambda r: r["repeat"])


def canonical_timeline(row):
    names = {}
    timeline = []
    for segment in sorted(row["speakerSegments"], key=lambda s: (s["start_ms"], s["end_ms"])):
        label = names.setdefault(segment["speaker_id"], len(names))
        timeline.append((segment["start_ms"], segment["end_ms"], label))
    return names, timeline


def measure_runs(before, after):
    left, left_rows = load_run(before)
    right, right_rows = load_run(after)
    for key in ("audioSha256", "audioSeconds", "modelArtifacts"):
        if not left[key] or left[key] != right[key]:
            raise ValueError(f"different inputs: {key}")
    configs = [
        {
            k: v
            for k, v in r["configuration"].items()
            if k
            not in (
                "embeddingThreads",
                "embeddingSpinning",
                "segmentationSpinning",
                "embeddingSpinDurationUs",
            )
        }
        for r in (left, right)
    ]
    if configs[0] != configs[1]:
        raise ValueError("different pipeline configuration")
    minimum_cosine = 1.0
    maximum_error = 0.0
    for a, b in zip(left_rows, right_rows):
        left_names, left_timeline = canonical_timeline(a)
        right_names, right_timeline = canonical_timeline(b)
        if a["speakerCount"] != b["speakerCount"] or left_timeline != right_timeline:
            raise ValueError("speaker timeline changed")
        if a["observations"] != b["observations"]:
            raise ValueError("embedding observation count changed")
        for x, y in zip(a["embeddingObservations"], b["embeddingObservations"]):
            origins = [
                (
                    o["startMs"],
                    o["endMs"],
                    names.get(f"speaker-{o['clusterId']}"),
                    o["quality"],
                    o["embeddingSpace"],
                )
                for o, names in ((x, left_names), (y, right_names))
            ]
            if any(origin[2] is None for origin in origins) or origins[0] != origins[1]:
                raise ValueError("embedding origin or space changed")
            u, v = x["vector"], y["vector"]
            if len(u) != 256 or len(v) != 256 or any(not math.isfinite(f) for f in u + v):
                raise ValueError("invalid embedding vector")
            norm = math.sqrt(sum(f * f for f in u) * sum(f * f for f in v))
            if not math.isfinite(norm) or norm <= 0:
                raise ValueError("invalid embedding norm")
            minimum_cosine = min(minimum_cosine, sum(p * q for p, q in zip(u, v)) / norm)
            maximum_error = max(maximum_error, max(abs(p - q) for p, q in zip(u, v)))
    if minimum_cosine < 0.99999 or maximum_error > 0.0001:
        raise ValueError("embedding numerical regression")
    baseline = statistics.median(r["wallSeconds"] for r in left_rows)
    candidate = statistics.median(r["wallSeconds"] for r in right_rows)
    speedup = 1 - candidate / baseline
    rss_ratio = right["peakRssBytes"] / left["peakRssBytes"]
    return {
        "qualityStatus": "baseline_equivalent",
        "baselineMedianSeconds": baseline,
        "candidateMedianSeconds": candidate,
        "speedup": speedup,
        "peakRssRatio": rss_ratio,
        "minimumEmbeddingCosine": minimum_cosine,
        "maximumEmbeddingAbsoluteError": maximum_error,
        "repeats": len(left_rows),
        "speakerTimeline": "identical after canonical identity mapping",
        "qualityScope": "baseline equivalence; not DER",
    }


def compare_runs(before, after):
    result = measure_runs(before, after)
    if result["speedup"] < 0.15 or result["peakRssRatio"] > 1.2:
        raise ValueError(f"runtime budget failed: {result}")
    return {"status": "passed", **result}


def compare_idle_cpu(before, after):
    result = measure_runs(before, after)
    left, _ = load_run(before)
    right, _ = load_run(after)
    if left["configuration"]["embeddingThreads"] != right["configuration"]["embeddingThreads"]:
        raise ValueError("CPU efficiency comparison changed thread count")
    times = [r.get("cpuSeconds") for r in (left, right)]
    if any(
        not isinstance(t, (int, float)) or isinstance(t, bool) or not math.isfinite(t) or t <= 0
        for t in times
    ):
        raise ValueError("missing or invalid process CPU time")
    saving = 1 - times[1] / times[0]
    wall_ratio = result["candidateMedianSeconds"] / result["baselineMedianSeconds"]
    if saving < 0.1 or wall_ratio > 1.05 or result["peakRssRatio"] > 1.1:
        raise ValueError(
            f"CPU efficiency budget failed: CPU saving={saving}, wall ratio={wall_ratio}"
        )
    return {"status": "passed", **result, "cpuTimeReduction": saving, "wallRatio": wall_ratio}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--criterion", choices=("speed", "quality", "idle-cpu"), default="speed")
    args = parser.parse_args()
    if args.output:
        inputs = {
            (directory / name).resolve()
            for directory in (args.baseline, args.candidate)
            for name in ("report.json", "prediction.jsonl")
        }
        if args.output.resolve() in inputs:
            raise ValueError("comparison output must not replace an input file")
        args.output.unlink(missing_ok=True)
    check = {"speed": compare_runs, "quality": measure_runs, "idle-cpu": compare_idle_cpu}[
        args.criterion
    ]
    text = json.dumps(check(args.baseline, args.candidate), indent=2) + "\n"
    if args.output:
        partial = args.output.with_name("." + args.output.name + ".part")
        partial.write_text(text)
        partial.replace(args.output)
    print(text, end="")


if __name__ == "__main__":
    main()
