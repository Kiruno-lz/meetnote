#!/usr/bin/env python3

import hashlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from fetch import download, verify_declared_files


ROOT = Path(__file__).resolve().parents[2]
FETCH_SCRIPT = Path(__file__).with_name("fetch.py")


class FakeResponse(io.BytesIO):
    def __init__(self, payload: bytes, status: int):
        super().__init__(payload)
        self.status = status


class FetchTests(unittest.TestCase):
    def test_cli_offline_skips_missing_file_from_repository_root(self):
        payload = b"not downloaded while offline"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest_path = root / "sources.json"
            manifest_path.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "datasets": [
                            {
                                "id": "sample",
                                "license": "TEST",
                                "licenseStatus": "allowed",
                                "files": [
                                    {
                                        "kind": "annotation",
                                        "relativePath": "sample/data.jsonl",
                                        "url": "https://example.invalid/data.jsonl",
                                        "bytes": len(payload),
                                        "sha256": hashlib.sha256(payload).hexdigest(),
                                    }
                                ],
                            }
                        ],
                        "excludedDatasets": [],
                    }
                ),
                encoding="utf-8",
            )
            output_dir = root / "cache"
            environment = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")

            completed = subprocess.run(
                [
                    sys.executable,
                    str(FETCH_SCRIPT),
                    "sample",
                    "--manifest",
                    str(manifest_path),
                    "--output-dir",
                    str(output_dir),
                    "--offline",
                ],
                cwd=ROOT,
                env=environment,
                check=True,
                capture_output=True,
                text=True,
            )

            report = json.loads(completed.stdout)
            self.assertEqual(completed.stderr, "")
            self.assertEqual(report["summary"]["skippedMissingCount"], 1)
            self.assertEqual(report["summary"]["verifiedCount"], 0)
            self.assertEqual(report["files"][0]["status"], "skipped-missing")
            self.assertFalse(report["files"][0]["verified"])
            self.assertFalse(output_dir.exists())

    def test_download_resumes_a_partial_file_and_verifies_checksum(self):
        payload = b"semantic-eval-source"
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "source.bin"
            destination.with_name("source.bin.part").write_bytes(payload[:8])
            item = {
                "relativePath": "dataset/source.bin",
                "url": "https://example.invalid/source.bin",
                "bytes": len(payload),
                "sha256": hashlib.sha256(payload).hexdigest(),
            }

            def respond(request, timeout):
                self.assertEqual(request.get_header("Range"), "bytes=8-")
                self.assertEqual(timeout, 60)
                return FakeResponse(payload[8:], 206)

            with patch("urllib.request.urlopen", side_effect=respond):
                outcome = download(item, destination)

            self.assertEqual(outcome.status, "resumed")
            self.assertEqual(outcome.http_status, 206)
            self.assertEqual(outcome.bytes_transferred, len(payload) - 8)
            self.assertEqual(destination.read_bytes(), payload)

    def test_converter_source_verification_includes_license_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            annotation = root / "annotation.jsonl"
            license_snapshot = root / "LICENSE"
            annotation.write_bytes(b"{}\n")
            license_snapshot.write_bytes(b"test license\n")
            manifest_path = root / "sources.json"
            manifest_path.write_text(
                json.dumps(
                    {
                        "datasets": [
                            {
                                "id": "sample",
                                "license": "TEST",
                                "licenseStatus": "allowed",
                                "files": [
                                    {
                                        "kind": "annotation",
                                        "relativePath": "sample/data.jsonl",
                                        "bytes": annotation.stat().st_size,
                                        "sha256": hashlib.sha256(
                                            annotation.read_bytes()
                                        ).hexdigest(),
                                    },
                                    {
                                        "kind": "license",
                                        "relativePath": "sample/LICENSE",
                                        "bytes": license_snapshot.stat().st_size,
                                        "sha256": hashlib.sha256(
                                            license_snapshot.read_bytes()
                                        ).hexdigest(),
                                    },
                                ],
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )
            dataset, evidence = verify_declared_files(
                manifest_path,
                "sample",
                {
                    "sample/data.jsonl": annotation,
                    "sample/LICENSE": license_snapshot,
                },
            )
            self.assertEqual(dataset["license"], "TEST")
            self.assertEqual(
                [item["kind"] for item in evidence], ["annotation", "license"]
            )

            license_snapshot.write_bytes(b"tampered license\n")
            with self.assertRaisesRegex(
                ValueError, "source verification failed for sample/LICENSE"
            ):
                verify_declared_files(
                    manifest_path,
                    "sample",
                    {
                        "sample/data.jsonl": annotation,
                        "sample/LICENSE": license_snapshot,
                    },
                )


if __name__ == "__main__":
    unittest.main()
