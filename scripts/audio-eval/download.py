#!/usr/bin/env python3
"""Download and receipt-check the real audio quality matrix inputs."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import tarfile
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = Path(__file__).with_name("datasets.json")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download(url: str, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    command = [
        "curl",
        "--fail",
        "--location",
        "--retry",
        "3",
        "--retry-all-errors",
        "--continue-at",
        "-",
        "--output",
        str(destination),
        url,
    ]
    subprocess.run(command, check=True)


def inspect_archive(path: Path) -> dict[str, object]:
    members: list[str] = []
    with tarfile.open(path, "r:gz") as archive:
        for member in archive:
            members.append(member.name)
            if len(members) >= 20_000:
                break
    lower = [member.lower() for member in members]
    return {
        "memberCountSampled": len(members),
        "membersSample": members[:100],
        "hasAudioMember": any(name.endswith((".wav", ".flac", ".sph")) for name in lower),
        "hasReferenceMember": any(
            name.endswith((".json", ".txt", ".rttm", ".stm", ".segments", ".trn"))
            for name in lower
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset", action="append", choices=("aishell4", "alimeeting", "chime6"))
    parser.add_argument("--split", action="append", help="limit selected dataset entries to these manifest splits")
    parser.add_argument("--extract", action="store_true")
    parser.add_argument(
        "--remove-archives",
        action="store_true",
        help="remove downloaded tar archives after extraction; receipt retains SHA-256",
    )
    parser.add_argument("--root", type=Path)
    args = parser.parse_args()

    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    selected = set(args.dataset or ("aishell4", "alimeeting", "chime6"))
    selected_splits = set(args.split or ())
    root = (args.root or ROOT / manifest["root"]).resolve()
    root.mkdir(parents=True, exist_ok=True)
    receipt_path = root / "download-receipt.json"
    existing: list[dict[str, object]] = []
    if receipt_path.is_file():
        existing = json.loads(receipt_path.read_text(encoding="utf-8")).get("items", [])
    by_key = {(item["id"], item["split"]): item for item in existing}

    def save_receipt() -> None:
        receipt_path.write_text(
            json.dumps(
                {
                    "schemaVersion": 1,
                    "manifest": str(MANIFEST),
                    "items": list(by_key.values()),
                },
                ensure_ascii=False,
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    receipts: list[dict[str, object]] = []
    for item in manifest["datasets"]:
        if item["id"] not in selected or (selected_splits and item["split"] not in selected_splits):
            continue
        archive = root / item["archive"]
        download(item["url"], archive)
        expected_bytes = item.get("expectedBytes")
        actual_bytes = archive.stat().st_size
        if expected_bytes is not None and actual_bytes != expected_bytes:
            raise ValueError(
                f"{item['id']} {item['split']} size mismatch: "
                f"expected {expected_bytes}, got {actual_bytes}"
            )
        record: dict[str, object] = {
            "id": item["id"],
            "split": item["split"],
            "url": item["url"],
            "path": str(archive),
            "bytes": actual_bytes,
            "sha256": sha256(archive),
            "downloadedAt": datetime.now(timezone.utc).isoformat(),
        }
        if args.extract:
            target = archive.with_suffix("").with_suffix("")
            target.mkdir(parents=True, exist_ok=True)
            with tarfile.open(archive, "r:gz") as tar:
                for member in tar.getmembers():
                    if member.issym() or member.islnk():
                        raise ValueError(f"archive member is a link: {member.name}")
                    destination = (target / member.name).resolve()
                    if target.resolve() not in destination.parents:
                        raise ValueError(f"archive member escapes extraction root: {member.name}")
                    tar.extract(member, target)
            record["extractedPath"] = str(target)
            record["archiveInspection"] = inspect_archive(archive)
            if args.remove_archives:
                archive.unlink()
                record["archiveRemoved"] = True
        receipts.append(record)
        by_key[(item["id"], item["split"])] = record
        save_receipt()
        print(json.dumps(record, ensure_ascii=False))

    save_receipt()
    print(f"receipt={receipt_path}")


if __name__ == "__main__":
    main()
