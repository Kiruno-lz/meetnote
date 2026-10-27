#!/usr/bin/env python3
"""Diagnose rough English ASR timing with MeetEval; never claim word alignment."""
import argparse
import importlib.metadata
import json
import math
from collections import defaultdict
from pathlib import Path

from asr_benchmark import load_benchmark_run, normalize, score, sha256, validate_cases


def temporal_score(case, row, collar):
    if not math.isfinite(collar) or collar < 0 or not case["language"].startswith("en"):
        raise ValueError("timing diagnostic requires English and a finite nonnegative collar")
    timing = case.get("wordTiming", {})
    rate = timing.get("sampleRateHz")
    if type(rate) is not int or rate <= 0 or not timing.get("source"):
        raise ValueError("missing word timing reference provenance or sample rate")
    if type(timing.get("manuallyVerified")) is not bool:
        raise ValueError("reference must declare whether word times were manually verified")
    source_samples = round(case["durationSeconds"] * rate)
    reference = []
    units = []
    identities = set()
    previous = -1
    for word in timing.get("words", []):
        start, end = word["startSample"], word["endSample"]
        if (
            type(start) is not int
            or type(end) is not int
            or not 0 <= start < end <= source_samples
            or start < previous
            or not word.get("id")
            or word["id"] in identities
        ):
            raise ValueError("invalid, clipped, unordered or duplicate reference word")
        previous = start
        identities.add(word["id"])
        tokens = normalize(word["text"], case["language"])
        if not tokens:
            raise ValueError("reference word has no lexical units")
        units.extend(tokens)
        reference.extend(
            {"words": token, "start_time": start / rate, "end_time": end / rate} for token in tokens
        )
    if not units or units != normalize(case["reference"], case["language"]):
        raise ValueError("word timing text does not match the frozen lexical reference")
    if "transcript" not in row:
        raise ValueError("prediction has no segment timing; cannot infer a whole-file substitute")
    hypothesis = []
    text = []
    previous = -1
    for segment in row["transcript"]:
        start, end = segment["startMs"], segment["endMs"]
        if (
            any(
                type(value) not in (int, float) or not math.isfinite(value)
                for value in (start, end)
            )
            or not 0 <= start <= end
            or end * rate > source_samples * 1000
            or start < previous
        ):
            raise ValueError("invalid or unordered hypothesis segment timing")
        previous = start
        tokens = normalize(segment["text"], case["language"])
        text.extend(tokens)
        if tokens:
            hypothesis.append(
                {"words": " ".join(tokens), "start_time": start / 1000, "end_time": end / 1000}
            )
    if row.get("text") is not None and normalize(row["text"], case["language"]) != text:
        raise ValueError("prediction text and timed segments disagree")
    from meeteval.wer.wer.time_constrained import time_constrained_siso_word_error_rate

    timed = time_constrained_siso_word_error_rate(
        reference,
        hypothesis,
        collar=collar,
        reference_pseudo_word_level_timing="none",
        hypothesis_pseudo_word_level_timing="character_based_points",
        reference_sort=False,
        hypothesis_sort=False,
    )
    literal = score(case["reference"], " ".join(text), case["language"])
    if timed.length != literal["referenceUnits"] or timed.errors < literal["errors"]:
        raise ValueError("temporal scoring disagrees with the frozen lexical units")
    return {
        "referenceUnits": int(timed.length),
        "literalErrors": literal["errors"],
        "timeConstrainedErrors": int(timed.errors),
        "insertions": int(timed.insertions),
        "deletions": int(timed.deletions),
        "substitutions": int(timed.substitutions),
    }


def summarize(cases, rows, collars):
    by_id = {case["id"]: case for case in cases}
    details = []
    groups = defaultdict(list)
    for collar in collars:
        for row in rows:
            case = by_id[row["caseId"]]
            detail = {"caseId": case["id"], "repeat": row["repeat"], "collarSeconds": collar}
            if row["status"] == "ok":
                try:
                    detail.update(temporal_score(case, row, collar))
                except ValueError as error:
                    raise ValueError(
                        f"{case['id']} repeat {row['repeat']} collar {collar}: {error}"
                    ) from error
                detail["status"] = "measured"
            else:
                detail.update(
                    status="unmeasured",
                    referenceUnits=len(normalize(case["reference"], case["language"])),
                )
            details.append(detail)
            groups[
                (case["language"], case.get("scenario", "native"), row["repeat"], collar)
            ].append(detail)
    summaries = []
    for (language, scenario, repeat, collar), items in sorted(groups.items()):
        failed = sum(item["status"] != "measured" for item in items)
        units = sum(item["referenceUnits"] for item in items)
        summaries.append(
            {
                "language": language,
                "scenario": scenario,
                "repeat": repeat,
                "collarSeconds": collar,
                "cases": len(items),
                "failures": failed,
                "referenceUnits": units,
                "literalErrorRate": (
                    None if failed else sum(i["literalErrors"] for i in items) / units
                ),
                "timeConstrainedErrorRate": (
                    None if failed else sum(i["timeConstrainedErrors"] for i in items) / units
                ),
            }
        )
    return summaries, details


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--collar", type=float, nargs="+", default=[0.5, 2.0])
    args = parser.parse_args()
    temporary = args.output.with_name(args.output.name + ".part")
    inputs = {
        (args.run / name).resolve()
        for name in ("report.json", "cases.json", "config.json", "prediction.jsonl")
    }
    if args.output.resolve() in inputs or temporary.resolve() in inputs:
        raise ValueError("timing output must not replace any input run artifact")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.unlink(missing_ok=True)
    if (
        not args.collar
        or len(set(args.collar)) != len(args.collar)
        or any(not math.isfinite(value) or value < 0 for value in args.collar)
    ):
        raise ValueError("collars must be finite and nonnegative")
    if importlib.metadata.version("meeteval") != "0.4.3":
        raise ValueError("install the pinned requirements-timing.txt environment")
    report, cases, rows = load_benchmark_run(args.run)
    if not report.get("predictionSha256"):
        raise ValueError("timing diagnostics require a prediction checksum")
    validate_cases(cases)
    groups, details = summarize(cases, rows, args.collar)
    complete = not any(group["failures"] for group in groups)
    result = {
        "schema": "meetnote.asr-temporal-diagnostic.v1",
        "metric": "time_constrained_siso_word_error_rate",
        "status": "complete" if complete else "incomplete",
        "qualityAcceptance": "not_assessed",
        "scope": "single-speaker rough timing diagnostic; not DER or word-alignment accuracy",
        "tool": "meeteval-0.4.3",
        "scorerSha256": sha256(Path(__file__)),
        "normalizerSha256": sha256(Path(__file__).with_name("asr_benchmark.py")),
        "referenceTiming": "dataset word intervals; provenance retained in cases.json",
        "hypothesisTiming": "character_based_points (approximate; not model word timestamps)",
        "collarsSeconds": args.collar,
        "inputReportSha256": sha256(args.run / "report.json"),
        "casesSha256": report["casesSha256"],
        "predictionSha256": report["predictionSha256"],
        "configuration": report["config"],
        "groups": groups,
        "cases": details,
    }
    try:
        temporary.write_text(json.dumps(result, indent=2, allow_nan=False) + "\n")
        temporary.replace(args.output)
    finally:
        temporary.unlink(missing_ok=True)
    if not complete:
        raise SystemExit(2)


if __name__ == "__main__":
    main()
