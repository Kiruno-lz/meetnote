#!/usr/bin/env python3
"""Run isolated production ASR adapters and score fixed B1 references exactly."""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import platform
import re
import os
import signal
import statistics
import subprocess
import sys
import time
import unicodedata
from collections import defaultdict
from pathlib import Path

from rapidfuzz.distance import Levenshtein
from whisper_normalizer.english import EnglishTextNormalizer

ENGLISH_SPEECH_NORMALIZER = EnglishTextNormalizer()


def normalize(text: str, language: str) -> list[str]:
    text = unicodedata.normalize("NFKC", text).casefold()
    # No number expansion, script conversion, LLM cleanup, or gold-dependent rules.
    text = "".join(
        (
            c
            if c.isspace() or unicodedata.category(c)[0] in "LNM"
            else "" if language.startswith("zh") or c in "'’" else " "
        )
        for c in text
    )
    return [c for c in text if not c.isspace()] if language.startswith("zh") else text.split()


def score(reference: str, prediction: str, language: str) -> dict:
    ref, hyp = normalize(reference, language), normalize(prediction, language)
    return {
        "errors": Levenshtein.distance(ref, hyp),
        "referenceUnits": len(ref),
        "method": "levenshtein",
        "metric": "cer" if language.startswith("zh") else "wer",
    }


def speech_normalized_score(reference: str, prediction: str) -> dict:
    ref = ENGLISH_SPEECH_NORMALIZER(reference).split()
    hyp = ENGLISH_SPEECH_NORMALIZER(prediction).split()
    return {
        "errors": Levenshtein.distance(ref, hyp),
        "referenceUnits": len(ref),
        "method": "levenshtein",
        "metric": "normalized_wer",
        "normalizer": "whisper-normalizer-0.1.15",
    }


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_prediction_rows(cases: list[dict], rows: list[dict], repeats: int) -> None:
    ids = [case["id"] for case in cases]
    if not ids or len(set(ids)) != len(ids) or type(repeats) is not int or repeats < 1:
        raise ValueError("invalid case inventory or repeat count")
    for row in rows:
        if type(row.get("repeat")) is not int:
            raise ValueError("prediction repeat must be an integer index")
        elapsed = row.get("wallSeconds")
        if type(elapsed) not in (int, float) or not 0 < elapsed <= sys.float_info.max:
            raise ValueError("prediction wallSeconds must be a finite positive measurement")
    expected = {(case_id, repeat) for case_id in ids for repeat in range(repeats)}
    actual = [(row["caseId"], row["repeat"]) for row in rows]
    if len(actual) != len(expected) or set(actual) != expected:
        raise ValueError("incomplete or duplicate prediction coverage for expected repeats")


def load_benchmark_run(path: Path, report: dict | None = None) -> tuple[dict, list, list]:
    if report is None:
        report = json.loads((path / "report.json").read_text())
    if report["failure"]:
        raise ValueError(f"incomplete run: {report['failure']}")
    case_bytes = (path / "cases.json").read_bytes()
    if hashlib.sha256(case_bytes).hexdigest() != report["casesSha256"]:
        raise ValueError("case manifest mismatch")
    prediction = path / "prediction.jsonl"
    prediction_bytes = prediction.read_bytes()
    if (
        report.get("predictionSha256")
        and hashlib.sha256(prediction_bytes).hexdigest() != report["predictionSha256"]
    ):
        raise ValueError("prediction checksum mismatch")
    cases = json.loads(case_bytes)
    rows = [json.loads(line) for line in prediction_bytes.splitlines()]
    validate_prediction_rows(cases, rows, report["config"].get("repeats", 1))
    return report, cases, rows


def stop_process_group(process: subprocess.Popen) -> None:
    # Benchmark output is disposable and has no shutdown protocol. Kill the
    # entire isolated group before reaping its leader, including native workers
    # that ignore SIGTERM. Do not race a second signal against reparented zombies.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def interrupted(_signum, _frame):
    raise KeyboardInterrupt


def merged_intervals(ranges: list, limit_ms: float) -> list[tuple[float, float]]:
    if not isinstance(ranges, list) or not math.isfinite(limit_ms) or limit_ms <= 0:
        raise ValueError("invalid timing intervals or duration")
    spans = []
    for span in ranges:
        if (
            not isinstance(span, (list, tuple))
            or len(span) != 2
            or any(type(value) not in (int, float) or not math.isfinite(value) for value in span)
        ):
            raise ValueError("invalid timing interval")
        start, end = span
        if not 0 <= start < end <= limit_ms:
            raise ValueError("timing interval outside audio")
        spans.append((start, end))
    merged = []
    for start, end in sorted(spans):
        if merged and start <= merged[-1][1]:
            merged[-1] = (merged[-1][0], max(end, merged[-1][1]))
        else:
            merged.append((start, end))
    return merged


def silence_overlap_ms(silence: list, speech: list) -> float:
    # Both inputs are sorted interval unions; never double-count overlapping
    # reference labels or overlapping ASR segments.
    i = j = 0
    overlap = 0
    while i < len(silence) and j < len(speech):
        a, b = silence[i], speech[j]
        overlap += max(0, min(a[1], b[1]) - max(a[0], b[0]))
        if a[1] <= b[1]:
            i += 1
        else:
            j += 1
    return overlap


def validate_cases(cases: list[dict]) -> None:
    seen = set()
    for case in cases:
        if case["id"] in seen:
            raise ValueError("duplicate case ID")
        seen.add(case["id"])
        if case["durationSeconds"] <= 0 or not normalize(case["reference"], case["language"]):
            raise ValueError("case needs positive duration and nonempty gold")
        if sha256(Path(case["audioPath"])) != case["sha256"]:
            raise ValueError(f"audio checksum mismatch: {case['id']}")
        if "knownSilenceRangesMs" in case:
            merged_intervals(case["knownSilenceRangesMs"], case["durationSeconds"] * 1000)


def summarize(cases: list[dict], rows: list[dict], expected_repeats: int = 1) -> list[dict]:
    validate_prediction_rows(cases, rows, expected_repeats)
    by_id = {case["id"]: case for case in cases}
    groups = defaultdict(list)
    for row in rows:
        key = row["caseId"], row["repeat"]
        case = by_id[key[0]]
        groups[(case["language"], case.get("scenario", "native"), key[1])].append((case, row))
    result = []
    for (language, scenario, repeat), items in sorted(groups.items()):
        errors = units = failures = 0
        walls = []
        duration = 0
        annotated = measured = 0
        silence_duration = silence_overlap = 0
        for case, row in items:
            walls.append(row["wallSeconds"])
            duration += case["durationSeconds"]
            if "knownSilenceRangesMs" in case:
                annotated += 1
                limit = case["durationSeconds"] * 1000
                silence = merged_intervals(case["knownSilenceRangesMs"], limit)
                silence_duration += sum(end - start for start, end in silence)
                if row["status"] == "ok":
                    if "transcript" not in row:
                        raise ValueError(
                            "annotated timing measurement requires transcript segments"
                        )
                    speech = merged_intervals(
                        [(segment["startMs"], segment["endMs"]) for segment in row["transcript"]],
                        limit,
                    )
                    silence_overlap += silence_overlap_ms(silence, speech)
                    measured += 1
            if row["status"] != "ok":
                failures += 1
                continue
            text = row.get("text")
            if text is None:
                separator = "" if language.startswith("zh") else " "
                text = separator.join(segment["text"] for segment in row["transcript"])
            scored = score(case["reference"], text, language)
            errors += scored["errors"]
            units += scored["referenceUnits"]
        successful_rate = errors / units if units else None
        result.append(
            {
                "language": language,
                "scenario": scenario,
                "repeat": repeat,
                "cases": len(items),
                "failures": failures,
                "generationTruncated": sum(
                    bool(row.get("generationTruncated")) for _, row in items
                ),
                "errors": errors,
                "referenceUnits": units,
                "metric": "cer" if language.startswith("zh") else "wer",
                "errorRate": None if failures else successful_rate,
                "successfulCasesErrorRate": successful_rate,
                "audioSeconds": duration,
                "wallSeconds": sum(walls),
                "rtf": sum(walls) / duration,
                "medianCaseSeconds": statistics.median(walls),
                "maxCaseSeconds": max(walls),
            }
        )
        if annotated:
            result[-1]["knownSilence"] = {
                "scope": "known_silence_only_not_word_alignment",
                "annotatedCases": annotated,
                "measuredCases": measured,
                "referenceMs": silence_duration,
                "overlapMs": silence_overlap if measured == annotated else None,
            }
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--runner", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=3600)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    report_path = args.output / "report.json"
    report_path.unlink(missing_ok=True)
    cases = json.loads(args.cases.read_text())
    validate_cases(cases)
    config = json.loads(args.config.read_text())
    model_path = Path(config["model"]["artifactPath"])
    artifact_paths = (
        sorted(p for p in model_path.rglob("*") if p.is_file())
        if model_path.is_dir()
        else [model_path]
    )
    artifacts = [
        {
            "filename": str(p.relative_to(model_path)) if model_path.is_dir() else p.name,
            "bytes": p.stat().st_size,
            "sha256": sha256(p),
        }
        for p in artifact_paths
    ]
    (args.output / "cases.json").write_bytes(args.cases.read_bytes())
    (args.output / "config.json").write_bytes(args.config.read_bytes())
    prediction = args.output / "prediction.jsonl"
    prediction.unlink(missing_ok=True)
    runner_sha = sha256(args.runner)
    command = [
        str(args.runner.resolve()),
        str(args.config.resolve()),
        str(args.cases.resolve()),
        str(prediction.resolve()),
    ]
    if args.runner.suffix == ".py":
        command.insert(0, sys.executable)
    # macOS time reports this process's high-water RSS; never reuse RUSAGE_CHILDREN
    # across configurations. GPU/ANE memory is not separately attributed.
    if platform.system() == "Darwin":
        command = ["/usr/bin/time", "-l", *command]
    started = time.monotonic()
    exit_code = None
    failure = None
    process = None
    previous_term = signal.signal(signal.SIGTERM, interrupted)
    try:
        with (args.output / "stderr.log").open("w") as log:
            try:
                process = subprocess.Popen(command, stdout=log, stderr=log, start_new_session=True)
                exit_code = process.wait(timeout=args.timeout)
            except subprocess.TimeoutExpired:
                failure = "timeout"
            except KeyboardInterrupt:
                failure = "cancelled"
            except OSError as error:
                failure = f"runner launch failed: {error}"
    finally:
        previous_int = signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        try:
            if process is not None:
                stop_process_group(process)
                exit_code = process.returncode
        finally:
            signal.signal(signal.SIGINT, previous_int)
            signal.signal(signal.SIGTERM, previous_term)
    try:
        rows = (
            [json.loads(line) for line in prediction.read_text().splitlines()]
            if prediction.exists()
            else []
        )
    except (OSError, ValueError) as error:
        rows = []
        failure = failure or f"invalid prediction output: {error}"
    expected_repeats = config.get("repeats", 1)
    if exit_code != 0 or len(rows) != len(cases) * expected_repeats:
        failure = failure or f"runner exit {exit_code}; incomplete output"
    # Preserve a crashed process and its partial predictions; never rank them as a full run.
    try:
        groups = summarize(cases, rows, expected_repeats) if not failure else []
    except ValueError as error:
        failure = str(error)
        groups = []
    log = (args.output / "stderr.log").read_text()
    rss = re.search(r"(\d+)\s+maximum resident set size", log)
    report = {
        "schema": "meetnote.asr-benchmark.v1",
        "config": config,
        "casesSha256": sha256(args.cases),
        "runnerSha256": runner_sha,
        "predictionSha256": sha256(prediction) if prediction.exists() else None,
        "scoringVersion": "meetnote.asr-scoring.v2",
        "platform": platform.platform(),
        "machine": platform.machine(),
        "modelLoading": (
            "per-call" if not config["model"]["backendId"].startswith("mlx") else "resident"
        ),
        "modelArtifacts": artifacts,
        "filesystemCache": "warmed-by-checksum-validation",
        "processSeconds": time.monotonic() - started,
        "peakRssBytes": int(rss[1]) if rss else None,
        "gpuMemory": "not_separately_measured",
        "exitCode": exit_code,
        "failure": failure,
        "groups": groups,
    }
    if platform.system() == "Darwin":
        report["hardware"] = {
            "cpu": subprocess.check_output(["sysctl", "-n", "machdep.cpu.brand_string"])
            .decode()
            .strip(),
            "cores": os.cpu_count(),
            "memoryBytes": int(subprocess.check_output(["sysctl", "-n", "hw.memsize"])),
        }
    report_part = args.output / ".report.json.part"
    report_part.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    report_part.replace(report_path)
    print(
        json.dumps(
            {"output": str(args.output), "failure": failure, "groups": groups}, ensure_ascii=False
        ),
        flush=True,
    )
    if failure:
        raise SystemExit(130 if failure == "cancelled" else 1)


if __name__ == "__main__":
    main()
