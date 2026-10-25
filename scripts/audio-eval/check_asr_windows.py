#!/usr/bin/env python3
"""Gate measured window changes against frozen quality budgets and short baselines."""
import argparse
import json
from pathlib import Path

from asr_benchmark import summarize, sha256


def load_run(path):
    report = json.loads((path / "report.json").read_text())
    if report["failure"]:
        raise ValueError(f"incomplete run: {report['failure']}")
    if sha256(path / "cases.json") != report["casesSha256"]:
        raise ValueError("case manifest mismatch")
    cases = json.loads((path / "cases.json").read_text())
    rows = [json.loads(line) for line in (path / "prediction.jsonl").read_text().splitlines()]
    return cases, rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--language", default="zh")
    parser.add_argument("--max-clean", type=float, required=True)
    parser.add_argument("--max-noise", type=float, required=True)
    args = parser.parse_args()
    cases, rows = load_run(args.run)
    groups = summarize(cases, rows)
    budgets = {"composed-clean": args.max_clean, "composed-noise20db": args.max_noise}
    checked = set()
    for group in groups:
        scenario = group["scenario"]
        if group["language"] != args.language or scenario not in budgets:
            continue
        checked.add(scenario)
        rate = group["errorRate"]
        if rate is None or rate > budgets[scenario]:
            raise ValueError(
                f"{scenario} repeat {group['repeat']}: {rate} exceeds {budgets[scenario]}"
            )
    if checked != set(budgets):
        raise ValueError("missing clean/noise measurements")
    by_id = {case["id"]: case for case in cases}
    for row in rows:
        if row["status"] != "ok":
            raise ValueError(f"failed case: {row['caseId']}")
        segments = row["transcript"]
        spans = [(segment["startMs"], segment["endMs"]) for segment in segments]
        if spans != sorted(spans) or any(
            start < 0 or end <= start or end > by_id[row["caseId"]]["durationSeconds"] * 1000 + 1
            for start, end in spans
        ):
            raise ValueError(f"invalid segment timing: {row['caseId']}")
    preserved = 0
    if args.baseline:
        before_cases, before_rows = load_run(args.baseline)
        before_cases = {case["id"]: case for case in before_cases}
        before_rows = {(row["caseId"], row["repeat"]): row for row in before_rows}
        for row in rows:
            case = by_id[row["caseId"]]
            before = before_cases[case["id"]]
            if (case["sha256"], case["reference"]) != (before["sha256"], before["reference"]):
                raise ValueError("baseline input or gold differs")
            if case["scenario"] == "native" and case["durationSeconds"] <= 30:
                previous = before_rows[(row["caseId"], row["repeat"])]
                if previous["status"] != "ok" or previous["transcript"] != row["transcript"]:
                    raise ValueError(f"short baseline changed: {case['id']}")
                preserved += 1
    print(
        json.dumps(
            {"status": "passed", "groups": groups, "shortCallsPreserved": preserved},
            ensure_ascii=False,
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
