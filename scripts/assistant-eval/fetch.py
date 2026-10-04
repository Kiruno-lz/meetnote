#!/usr/bin/env python3
"""Fetch text-only Assistant evaluation sources with checksums and resume."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = Path(__file__).with_name("sources.json")
DEFAULT_OUTPUT = ROOT / "eval-data/assistant/data"


@dataclass(frozen=True)
class DownloadOutcome:
    status: str
    http_status: int
    bytes_transferred: int


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def observe(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {"observedBytes": None, "observedSha256": None, "exists": False}
    return {
        "observedBytes": path.stat().st_size,
        "observedSha256": sha256(path),
        "exists": True,
    }


def verified(path: Path, item: dict[str, Any]) -> bool:
    observation = observe(path)
    return (
        observation["exists"]
        and observation["observedBytes"] == item["bytes"]
        and observation["observedSha256"] == item["sha256"]
    )


def load_dataset(manifest_path: Path, dataset_id: str) -> dict[str, Any]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    datasets = {dataset["id"]: dataset for dataset in manifest["datasets"]}
    if dataset_id not in datasets:
        raise ValueError(f"unknown or excluded dataset: {dataset_id}")
    dataset = datasets[dataset_id]
    if dataset["licenseStatus"] != "allowed":
        raise ValueError(f"dataset {dataset_id} is not license-approved")
    return dataset


def verify_declared_files(
    manifest_path: Path,
    dataset_id: str,
    paths_by_relative_name: dict[str, Path],
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Verify converter inputs against the pinned source and license manifest."""
    dataset = load_dataset(manifest_path, dataset_id)
    evidence = []
    for item in dataset["files"]:
        relative_name = item["relativePath"]
        path = paths_by_relative_name.get(relative_name)
        if path is None:
            raise ValueError(
                f"no local path supplied for declared source {relative_name}"
            )
        observation = observe(path)
        if (
            not observation["exists"]
            or observation["observedBytes"] != item["bytes"]
            or observation["observedSha256"] != item["sha256"]
        ):
            raise ValueError(
                f"source verification failed for {relative_name}: "
                f"bytes={observation['observedBytes']} sha256={observation['observedSha256']}"
            )
        evidence.append(
            {
                "kind": item["kind"],
                "relativePath": relative_name,
                "bytes": observation["observedBytes"],
                "sha256": observation["observedSha256"],
            }
        )
    return dataset, evidence


def download(item: dict[str, Any], destination: Path) -> DownloadOutcome:
    destination.parent.mkdir(parents=True, exist_ok=True)
    partial = destination.with_name(destination.name + ".part")
    start = partial.stat().st_size if partial.exists() else 0
    headers = {"User-Agent": "MeetNote-Assistant-Eval/1"}
    if start:
        headers["Range"] = f"bytes={start}-"
    request = urllib.request.Request(item["url"], headers=headers)
    try:
        response = urllib.request.urlopen(request, timeout=60)
    except urllib.error.HTTPError as error:
        if error.code == 416 and partial.exists():
            if verified(partial, item):
                os.replace(partial, destination)
                return DownloadOutcome("resumed", 416, 0)
            partial.unlink()
            return download(item, destination)
        raise
    with response:
        http_status = response.status
        append = start > 0 and response.status == 206
        if start and not append:
            start = 0
        mode = "ab" if append else "wb"
        before = partial.stat().st_size if append else 0
        with partial.open(mode) as handle:
            shutil.copyfileobj(response, handle, length=1024 * 1024)
        transferred = partial.stat().st_size - before
    if not verified(partial, item):
        observed_size = partial.stat().st_size
        observed_hash = sha256(partial)
        raise RuntimeError(
            f"checksum mismatch for {item['relativePath']}: "
            f"bytes={observed_size} sha256={observed_hash}"
        )
    os.replace(partial, destination)
    return DownloadOutcome(
        "downloaded" if start == 0 else "resumed", http_status, transferred
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "dataset", nargs="*", help="Dataset IDs; defaults to every allowed dataset"
    )
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument(
        "--offline",
        action="store_true",
        help="Verify cached files and skip missing files",
    )
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    datasets = {dataset["id"]: dataset for dataset in manifest["datasets"]}
    selected_ids = args.dataset or sorted(datasets)
    unknown = sorted(set(selected_ids) - set(datasets))
    if unknown:
        parser.error(f"unknown or excluded dataset: {', '.join(unknown)}")
    results = []
    for dataset_id in selected_ids:
        dataset = datasets[dataset_id]
        if dataset["licenseStatus"] != "allowed":
            raise RuntimeError(f"dataset {dataset_id} is not license-approved")
        for item in dataset["files"]:
            destination = args.output_dir / item["relativePath"]
            if verified(destination, item):
                status = "verified"
                http_status = None
                bytes_transferred = 0
            elif args.offline:
                status = "skipped-missing"
                http_status = None
                bytes_transferred = 0
            else:
                outcome = download(item, destination)
                status = outcome.status
                http_status = outcome.http_status
                bytes_transferred = outcome.bytes_transferred
            observation = observe(destination)
            results.append(
                {
                    "dataset": dataset_id,
                    "kind": item["kind"],
                    "path": str(destination),
                    "url": item["url"],
                    "httpStatus": http_status,
                    "bytesTransferred": bytes_transferred,
                    "expectedBytes": item["bytes"],
                    "observedBytes": observation["observedBytes"],
                    "expectedSha256": item["sha256"],
                    "observedSha256": observation["observedSha256"],
                    "verified": verified(destination, item),
                    "status": status,
                }
            )
    print(
        json.dumps(
            {
                "schemaVersion": 1,
                "offline": args.offline,
                "files": results,
                "summary": {
                    "selectedDatasets": selected_ids,
                    "fileCount": len(results),
                    "verifiedCount": sum(result["verified"] for result in results),
                    "skippedMissingCount": sum(
                        result["status"] == "skipped-missing" for result in results
                    ),
                    "bytesTransferred": sum(
                        result["bytesTransferred"] for result in results
                    ),
                },
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
