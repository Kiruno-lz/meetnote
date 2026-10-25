#!/usr/bin/env python3
"""Run isolated production ASR adapters and score fixed B1 references exactly."""
from __future__ import annotations

import argparse
import hashlib
import json
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


def summarize(cases: list[dict], rows: list[dict]) -> list[dict]:
    by_id = {case["id"]: case for case in cases}
    seen = set()
    groups = defaultdict(list)
    for row in rows:
        key = row["caseId"], row["repeat"]
        if key in seen or key[0] not in by_id:
            raise ValueError("duplicate or unknown prediction")
        seen.add(key)
        case = by_id[key[0]]
        groups[(case["language"], case.get("scenario", "native"), key[1])].append((case, row))
    repeats = {row["repeat"] for row in rows}
    if not repeats or seen != {(id, rep) for id in by_id for rep in repeats}:
        raise ValueError("incomplete prediction coverage")
    result = []
    for (language, scenario, repeat), items in sorted(groups.items()):
        errors = units = failures = 0
        walls = []
        duration = 0
        for case, row in items:
            walls.append(row["wallSeconds"])
            duration += case["durationSeconds"]
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
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--runner", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=3600)
    args = parser.parse_args()
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
    args.output.mkdir(parents=True, exist_ok=True)
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
    with (args.output / "stderr.log").open("w") as log:
        try:
            process = subprocess.Popen(command, stdout=log, stderr=log, start_new_session=True)
            exit_code = process.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            failure = "timeout"
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
    rows = (
        [json.loads(line) for line in prediction.read_text().splitlines()]
        if prediction.exists()
        else []
    )
    expected_repeats = config.get("repeats", 1)
    if exit_code != 0 or len(rows) != len(cases) * expected_repeats:
        failure = failure or f"runner exit {exit_code}; incomplete output"
    # Preserve a crashed process and its partial predictions; never rank them as a full run.
    try:
        groups = summarize(cases, rows) if not failure else []
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
    (args.output / "report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    )
    print(
        json.dumps(
            {"output": str(args.output), "failure": failure, "groups": groups}, ensure_ascii=False
        ),
        flush=True,
    )
    if failure:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
