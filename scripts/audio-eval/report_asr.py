#!/usr/bin/env python3
"""Generate comparable B1 tables and a compact receipt from measured runs."""
import argparse
import collections
import fnmatch
import hashlib
import json
import math
import statistics
from datetime import datetime, timezone
from pathlib import Path
from asr_benchmark import score, speech_normalized_score, load_benchmark_run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--date", default=datetime.now(timezone.utc).date().isoformat())
    parser.add_argument(
        "--include", action="append", help="Include run names matching any glob; default all"
    )
    args = parser.parse_args()
    root = args.root.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    receipt = []
    detail = []
    hardware = None
    for report_path in sorted((root / "runs").glob("*/report.json")):
        name = report_path.parent.name
        if args.include and not any(fnmatch.fnmatchcase(name, pattern) for pattern in args.include):
            continue
        report = json.loads(report_path.read_text())
        hardware = report.get("hardware", hardware)
        # Each run retains its exact case list, including language/smoke subsets.
        case_path = report_path.parent / "cases.json"
        if not case_path.exists():
            raise ValueError(f"missing case receipt: {name}")
        cases = {c["id"]: c for c in json.loads(case_path.read_text())}
        if hashlib.sha256(case_path.read_bytes()).hexdigest() != report["casesSha256"]:
            raise ValueError(f"case manifest mismatch: {name}")
        config = report["config"].copy()
        config["model"] = dict(config["model"])
        config["model"].pop("artifactPath", None)
        if report["failure"]:
            receipt.append(
                {
                    "name": name,
                    "failure": report["failure"],
                    "groups": [],
                    "configuration": config,
                    "runnerSha256": report["runnerSha256"],
                    "casesSha256": report["casesSha256"],
                    "peakRssBytes": report["peakRssBytes"],
                }
            )
            continue
        _, _, predictions = load_benchmark_run(report_path.parent, report)
        groups = collections.defaultdict(list)
        for row in predictions:
            case = cases[row["caseId"]]
            language = case["language"]
            text = row.get("text")
            if text is None:
                text = ("" if language.startswith("zh") else " ").join(
                    t["text"] for t in row.get("transcript", [])
                )
            scoring = score(case["reference"], text, language) if row["status"] == "ok" else None
            item = {
                "caseId": case["id"],
                "repeat": row["repeat"],
                "language": language,
                "scenario": case.get("scenario", "native"),
                "status": row["status"],
                "audioSeconds": case["durationSeconds"],
                "wallSeconds": row["wallSeconds"],
                "scoring": scoring,
                "audioSha256": case["sha256"],
                "referenceSha256": hashlib.sha256(case["reference"].encode()).hexdigest(),
                "predictionSha256": hashlib.sha256(text.encode()).hexdigest(),
                "generationTruncated": row.get("generationTruncated", False),
            }
            if language == "en" and row["status"] == "ok":
                item["speechNormalizedScoring"] = speech_normalized_score(case["reference"], text)
            if row.get("modelLoadSeconds") is not None:
                item["modelLoadSeconds"] = row["modelLoadSeconds"]
            groups[(language, case.get("scenario", "native"))].append(item)
            detail.append({"run": name, **item})
        compact = []
        for (language, scenario), items in sorted(groups.items()):
            failures = sum(i["status"] != "ok" for i in items)
            errors = sum(i["scoring"]["errors"] for i in items if i["scoring"])
            units = sum(i["scoring"]["referenceUnits"] for i in items if i["scoring"])
            normalized_errors = sum(
                i.get("speechNormalizedScoring", {}).get("errors", 0) for i in items
            )
            normalized_units = sum(
                i.get("speechNormalizedScoring", {}).get("referenceUnits", 0) for i in items
            )
            walls = sorted(i["wallSeconds"] for i in items)
            compact.append(
                {
                    "language": language,
                    "scenario": scenario,
                    "cases": len({i["caseId"] for i in items}),
                    "repeats": len({i["repeat"] for i in items}),
                    "failures": failures,
                    "metric": "CER" if language.startswith("zh") else "WER",
                    "errorRate": None if failures or not units else errors / units,
                    "normalizedWer": (
                        None
                        if failures or not normalized_units
                        else normalized_errors / normalized_units
                    ),
                    "errors": errors,
                    "referenceUnits": units,
                    "rtf": sum(walls) / sum(i["audioSeconds"] for i in items),
                    "medianSeconds": statistics.median(walls),
                    "p95Seconds": walls[math.ceil(0.95 * len(walls)) - 1],
                    "truncated": sum(i["generationTruncated"] for i in items),
                }
            )
        receipt.append(
            {
                "name": name,
                "configuration": config,
                "groups": compact,
                "peakRssBytes": report["peakRssBytes"],
                "initialSessionLoadSeconds": (
                    predictions[0].get("modelLoadSeconds") if predictions else None
                ),
                "modelLoading": report["modelLoading"],
                "runtimeDevice": (
                    predictions[0].get("effectiveDevice", "not-instrumented")
                    if predictions
                    else None
                ),
                "runtimeThreads": (
                    predictions[0].get("threads", config["threads"]) if predictions else None
                ),
                "modelArtifacts": report.get("modelArtifacts", []),
                "casesSha256": report["casesSha256"],
                "runnerSha256": report["runnerSha256"],
                "failure": None,
            }
        )
    hardware_text = (
        f"{hardware['cpu']}, {hardware['cores']} cores, {hardware['memoryBytes']/2**30:.0f} GiB"
        if hardware
        else "hardware receipt unavailable"
    )
    artifact = {
        "schema": "meetnote.asr-comparison.v1",
        "date": args.date,
        "hardware": hardware,
        "normalization": "meetnote.asr-scoring.v2: NFKC + casefold; Chinese remove punctuation, English punctuation to spaces except apostrophes; no script conversion; report literal WER and whisper-normalizer-0.1.15 normalized WER separately",
        "qualityScope": "B1 ASR; timestamp error, DER, identity and semantic quality not assessed",
        "scorerArtifacts": {
            name: hashlib.sha256((Path(__file__).parent / name).read_bytes()).hexdigest()
            for name in (
                "asr_benchmark.py",
                "report_asr.py",
                "requirements.txt",
                "requirements-mlx.txt",
            )
        },
        "runs": receipt,
    }
    details = json.dumps({"caseResults": detail}, ensure_ascii=False, indent=2) + "\n"
    details_path = root / "report-details" / args.output.resolve().name / "comparison-details.json"
    details_path.parent.mkdir(parents=True, exist_ok=True)
    details_path.write_text(details)
    artifact["caseResultsPath"] = str(details_path.relative_to(root))
    artifact["caseResultsSha256"] = hashlib.sha256(details.encode()).hexdigest()
    (args.output / "asr-comparison.json").write_text(
        json.dumps(artifact, ensure_ascii=False, indent=2) + "\n"
    )
    lines = [
        "# 本机 ASR 模型与后端评测",
        "",
        f"日期：{args.date}。硬件：{hardware_text}；Rust runner 使用 release 构建。",
        "",
        "文本评分为精确 Levenshtein；中文 CER、英文原样 WER，按参考单位加权。英文同时给出 whisper-normalizer 0.1.15 的 normalized WER（数字、缩写等口语格式归一），原样错误率保留，不将格式差异隐去。两次执行共用固定输入。原生短片段、构造长片段、AMI 标注裁剪片段分别列出。RTF 越低越快；RSS 是每个独立配置进程的峰值，不能视为独立 GPU 显存。",
        "",
        "原生后端每次调用重新加载模型，耗时包含加载与解码；MLX 是驻留 session，表中为推理调用耗时，加载时间保存在逐项 receipt，不能把两种加载策略当成同一部署性能。模型 checksum 校验会预热文件缓存，本报告不测磁盘冷启动。",
        "",
        "| 配置 | 语言/场景 | 样本×重复 | CER/WER | normalized WER | RTF | 调用中位/P95 秒 | 进程峰值 MiB | 失败 |",
        "|---|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for run in receipt:
        if run["failure"]:
            lines.append(f"| {run['name']} | 执行失败 | — | — | — | — | — | — | {run['failure']} |")
            continue
        for g in run["groups"]:
            rate = "—" if g["errorRate"] is None else f"{100*g['errorRate']:.2f}%"
            rss = "—" if run["peakRssBytes"] is None else f"{run['peakRssBytes']/2**20:.0f}"
            normalized = "—" if g["normalizedWer"] is None else f"{100*g['normalizedWer']:.2f}%"
            lines.append(
                f"| {run['name']} | {g['language']}/{g['scenario']} | {g['cases']}×{g['repeats']} | {rate} | {normalized} | {g['rtf']:.4f} | {g['medianSeconds']:.3f}/{g['p95Seconds']:.3f} | {rss} | {g['failures']} |"
            )
    lines += [
        "",
        "模型版本、设备、线程、加载策略、模型文件 SHA-256 与分组结果见同目录 `asr-comparison.json`；逐样本评分及耗时保留在忽略的评测运行目录 `comparison-details.json`，其 SHA-256 写入汇总。原始预测与音频不提交。",
        "",
        "FLEURS 来源固定为 google/fleurs `70bb2e84b976b7e960aa89f1c648e09c59f894dd` 的中英文 test；AMI 为 ES2002a Mix-Headset 和 manual 1.6.2 官方词标注，二者均为 CC-BY-4.0。AMI 使用官方非重叠片段与标注边界裁剪，不能证明自动 VAD 或重叠会议能力。",
        "",
        "两次执行的波动不能替代统计显著性。数据含朗读与少量会话，不代表小时级真实会议、说话人分离、词时间戳或发布质量验收。",
    ]
    (args.output / "model-comparison.md").write_text("\n".join(lines) + "\n")
    print(f"Generated {len(receipt)} runs / {len(detail)} measured calls")


if __name__ == "__main__":
    main()
