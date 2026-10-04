#!/usr/bin/env python3
"""Verify checked-in Assistant skills and semantic-evaluation evidence."""

from __future__ import annotations

import argparse
import json
import tempfile
from collections import Counter
from pathlib import Path

from evaluate import (
    compare_with_baselines,
    evaluate_records,
    oracle_predictions,
    read_jsonl,
    validate_gold_record,
)
from generate_synthetic import (
    CORPUS_NAME,
    FEATURES,
    MANIFEST_NAME,
    generate,
    write_outputs,
)
from review import EvidenceError, sha256_file, summarize, validate_manifest
from semantic_schema import SemanticSchemaError, validate_semantic_fixture


ROOT = Path(__file__).resolve().parents[2]
FIXTURE_DIR = ROOT / "crates/application/tests/fixtures/assistant"
SKILL_DIR = ROOT / "resources/assistant/skills"
SOURCE_MANIFEST = ROOT / "scripts/assistant-eval/sources.json"
REVIEW_MANIFEST = ROOT / "scripts/assistant-eval/review-manifest.json"
MODEL_MATRIX = ROOT / "scripts/assistant-eval/model-matrix.json"
SEMANTIC_FIXTURE = Path(__file__).with_name("fixtures") / "cross-scope.json"
EXPECTED_SKILLS = {
    "ask": ["list_documents", "read_document"],
    "organize": ["read_document", "update_document"],
    "topic-partition": ["read_document", "create_semantic_children"],
    "topic-extract": ["read_document", "create_semantic_children"],
    "move": ["list_documents", "read_document", "move_document"],
}


def parse_frontmatter(path: Path) -> dict[str, object]:
    lines = path.read_text(encoding="utf-8").splitlines()
    if not lines or lines[0] != "---":
        raise AssertionError(f"{path}: missing frontmatter")
    try:
        end = lines.index("---", 1)
    except ValueError as error:
        raise AssertionError(f"{path}: unterminated frontmatter") from error
    values: dict[str, object] = {}
    current_list: list[str] | None = None
    for line in lines[1:end]:
        if line.startswith("  - ") and current_list is not None:
            current_list.append(line[4:])
        elif ":" in line:
            key, raw = line.split(":", 1)
            raw = raw.strip()
            if raw:
                values[key] = int(raw) if raw.isdigit() else raw
                current_list = None
            else:
                current_list = []
                values[key] = current_list
        elif line.strip():
            raise AssertionError(f"{path}: unsupported frontmatter line: {line}")
    if not any(line.strip() for line in lines[end + 1 :]):
        raise AssertionError(f"{path}: empty instructions")
    return values


def verify_skills() -> None:
    actual_dirs = {path.name for path in SKILL_DIR.iterdir() if path.is_dir()}
    assert actual_dirs == set(EXPECTED_SKILLS), (actual_dirs, set(EXPECTED_SKILLS))
    for skill_id, tools in EXPECTED_SKILLS.items():
        metadata = parse_frontmatter(SKILL_DIR / skill_id / "SKILL.md")
        assert set(metadata) == {"id", "version", "allowed_tools"}, metadata
        assert metadata["id"] == skill_id
        assert metadata["version"] == 1
        assert metadata["allowed_tools"] == tools


def verify_source_manifest() -> None:
    manifest = json.loads(SOURCE_MANIFEST.read_text(encoding="utf-8"))
    assert manifest["schemaVersion"] == 1
    datasets = {dataset["id"]: dataset for dataset in manifest["datasets"]}
    assert set(datasets) == {"qmsum", "ami"}
    relative_paths: set[str] = set()
    for dataset in datasets.values():
        assert dataset["licenseStatus"] == "allowed"
        assert dataset["license"]
        assert dataset["version"]
        assert dataset["homepage"].startswith("https://")
        assert dataset["scope"]
        assert dataset["splits"]
        assert dataset["baseline"]["evaluator"] == "scripts/assistant-eval/evaluate.py"
        assert set(dataset["baseline"]["required"]) == {"no-split", "equal-length"}
        assert dataset["baseline"]["comparison"] == "candidate must improve boundaryF1 and reduce Pk and WindowDiff"
        assert dataset["files"]
        assert any(item["kind"] == "annotation" for item in dataset["files"])
        assert any(item["kind"] == "license" for item in dataset["files"])
        assert sum(item["kind"] == "license" for item in dataset["files"]) == 1
        for item in dataset["files"]:
            assert item["relativePath"] not in relative_paths
            relative_paths.add(item["relativePath"])
            assert item["url"].startswith("https://")
            assert item["bytes"] > 0
            assert len(item["sha256"]) == 64
            assert item["sha256"] == item["sha256"].lower()
            int(item["sha256"], 16)
    excluded = {dataset["id"]: dataset for dataset in manifest["excludedDatasets"]}
    superdialseg = excluded["superdialseg"]
    assert superdialseg["enabled"] is False
    assert superdialseg["licenseStatus"] == "unverified-upstream-licenses"


def verify_review_manifest(records: list[dict[str, object]], corpus_sha256: str) -> None:
    manifest = json.loads(REVIEW_MANIFEST.read_text(encoding="utf-8"))
    sample_ids = validate_manifest(manifest)
    assert manifest["corpusSha256"] == corpus_sha256
    by_id = {record["scenarioId"]: record for record in records}
    for sample in manifest["samples"]:
        record = by_id[sample["scenarioId"]]
        assert sample["expectedMode"] == record["gold"]["allowed_mode"]
        assert sample["reviewer"] is None
    assert len(sample_ids) == 24


def verify_model_matrix() -> None:
    matrix = json.loads(MODEL_MATRIX.read_text(encoding="utf-8"))
    assert matrix["schemaVersion"] == 1
    assert matrix["status"] == "pending_external_model"
    assert matrix["protocols"] == [
        "openaiChatCompletions",
        "anthropicMessages",
        "openaiResponses",
    ]
    assert len(matrix["cases"]) == 10
    assert len(set(matrix["cases"])) == len(matrix["cases"])
    assert matrix["requiredEvidence"] == [
        "predictionFile",
        "runMetadata",
        "structuredErrorOrReadback",
        "mutationReceipts",
    ]
    assert matrix["runs"] == []
    assert matrix["runContract"]["metadataRequired"] == [
        "runId", "caseId", "model", "promptSha256", "budget", "resources", "elapsedMs", "predictionSource"
    ]
    assert matrix["runContract"]["receiptRequired"] == [
        "runId", "requestSha256", "predictionSha256", "readbackSha256", "mutationReceipts"
    ]
    assert matrix["runContract"]["readbackRequired"] == [
        "runId", "predictionSha256", "metadataSha256", "mutationReceipts", "validation"
    ]
    assert matrix["predictionContracts"]["validator"]["correctionRequired"] == [
        "errorCode", "correctedOperation", "retry"
    ]
    assert matrix["predictionContracts"]["validator"]["retryMustBe"] is False
    assert matrix["semanticFixtures"] == [
        {
            "id": "cross-scope",
            "path": "scripts/assistant-eval/fixtures/cross-scope.json",
            "requiredCases": [
                "cross-block-partition",
                "cross-document-extract",
                "single-topic-none",
                "cross-block-second-pass",
            ],
            "independentModelEvidence": False,
        }
    ]


def verify_semantic_fixture() -> dict[str, object]:
    """Check cross-scope identity, coverage, re-splitting, and negative cases."""
    try:
        fixture = json.loads(SEMANTIC_FIXTURE.read_text(encoding="utf-8"))
        return validate_semantic_fixture(fixture)
    except (OSError, json.JSONDecodeError, SemanticSchemaError) as error:
        raise AssertionError(f"semantic fixture verification failed: {error}") from error


def indices_for(
    children: list[dict[str, object]], index_by_id: dict[str, int]
) -> list[int]:
    covered: list[int] = []
    for child in children:
        start = index_by_id[str(child["start_segment_id"])]
        end = index_by_id[str(child["end_segment_id"])]
        assert start <= end
        covered.extend(range(start, end + 1))
        evidence = [index_by_id[str(value)] for value in child["evidence_segment_ids"]]
        assert evidence
        assert all(start <= value <= end for value in evidence)
    return covered


def verify_record(record: dict[str, object]) -> None:
    transcript = record["record"]["state"]["results"]["transcript"]
    ids = [segment["id"] for segment in transcript]
    assert len(ids) == len(set(ids)) == 8
    assert all(segment["start_ms"] < segment["end_ms"] for segment in transcript)
    assert all(
        left["end_ms"] <= right["start_ms"]
        for left, right in zip(transcript, transcript[1:])
    )
    index_by_id = {segment_id: index for index, segment_id in enumerate(ids)}
    gold = record["gold"]
    covered = indices_for(gold["children"], index_by_id)
    assert len(covered) == len(set(covered))
    mode = gold["allowed_mode"]
    if mode == "partition":
        assert covered == list(range(len(ids)))
        assert gold["uncovered_ranges"] == []
    elif mode == "extract":
        uncovered = indices_for(
            [
                dict(value, evidence_segment_ids=[value["start_segment_id"]])
                for value in gold["uncovered_ranges"]
            ],
            index_by_id,
        )
        assert sorted(covered + uncovered) == list(range(len(ids)))
    else:
        assert mode == "none" and not covered
    if "second_pass" in gold:
        parent = gold["children"][gold["second_pass"]["parent_child_index"]]
        parent_start = index_by_id[parent["start_segment_id"]]
        parent_end = index_by_id[parent["end_segment_id"]]
        nested = indices_for(gold["second_pass"]["children"], index_by_id)
        assert nested == list(range(parent_start, parent_end + 1))


def verify_oracle_regression(
    records: list[dict[str, object]], sample_ids: list[str]
) -> dict[str, object]:
    """Run the gold-copy oracle as evaluator/structure regression only."""
    by_id = {record["scenarioId"]: record for record in records}
    selected = [by_id[scenario_id] for scenario_id in sample_ids]
    predictions = oracle_predictions(selected)
    report = evaluate_records(selected, predictions)
    assert report["exampleCount"] == len(sample_ids)
    assert report["missingScenarioIds"] == []
    assert report["unexpectedScenarioIds"] == []
    assert report["exactMatchRate"] == 1.0
    assert report["structuralValidityRate"] == 1.0
    assert report["boundaryF1"] == 1.0
    assert report["Pk"] == 0.0
    assert report["WindowDiff"] == 0.0
    assert report["evidenceValidityRate"] == 1.0
    assert report["hierarchyValidityRate"] == 1.0
    assert report["partitionCoverageRate"] == 1.0
    assert report["extractGapReportValidityRate"] == 1.0
    comparison = compare_with_baselines(selected, predictions)
    assert comparison["outperforms"] == {
        "noSplitOnAllMetrics": True,
        "equalLengthOnAllMetrics": True,
    }
    return {
        "status": "structural_regression",
        "predictionSource": "oracle-gold-copy",
        "sampleCount": len(sample_ids),
        "metrics": {
            key: report[key]
            for key in ("exactMatchRate", "structuralValidityRate", "boundaryF1", "Pk", "WindowDiff")
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", type=Path, default=FIXTURE_DIR / CORPUS_NAME)
    parser.add_argument("--prediction", type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if (args.prediction is None) != (args.report is None):
        parser.error("--prediction and --report must be supplied together")

    try:
        verify_skills()
        verify_source_manifest()
        records = read_jsonl(args.corpus)
        fixture_manifest = json.loads((FIXTURE_DIR / MANIFEST_NAME).read_text(encoding="utf-8"))
        corpus_sha256 = sha256_file(args.corpus)
        assert len(records) == fixture_manifest["documentCount"] == 48
        assert sum(bool(record["smoke"]) for record in records) == fixture_manifest["smokeCount"] == 8
        assert corpus_sha256 == fixture_manifest["corpusSha256"]
        dimensions = Counter(
            (
                record["dimensions"]["scene"],
                record["dimensions"]["language"],
                record["dimensions"]["structure"],
            )
            for record in records
        )
        assert len(dimensions) == 48 and set(dimensions.values()) == {1}
        assert set().union(*(set(record["features"]) for record in records)) == set(FEATURES)
        for record in records:
            verify_record(record)
            validation = validate_gold_record(record)
            assert validation.valid, (record["scenarioId"], validation.errors)
        verify_review_manifest(records, corpus_sha256)
        verify_model_matrix()
        semantic_fixture = verify_semantic_fixture()
        assert set(record["negativeCase"]["expected_error_code"] for record in records) == {
            "RANGE_INVALID_ORDER",
            "RANGE_OVERLAP",
            "RANGE_COVERAGE_GAP",
            "SEGMENT_NOT_FOUND",
            "TREE_MAX_DEPTH_EXCEEDED",
            "DOCUMENT_REVISION_CONFLICT",
            "TREE_CYCLE",
            "TREE_CROSS_AUDIO_ASSET",
            "DOCUMENT_ACCESS_DENIED",
            "DOCUMENT_FIELD_READ_ONLY",
            "TOOL_ARGUMENTS_INVALID",
            "TOOL_NOT_ALLOWED",
        }
        review_manifest = json.loads(REVIEW_MANIFEST.read_text(encoding="utf-8"))
        sample_ids = validate_manifest(review_manifest)
        oracle_regression = verify_oracle_regression(records, sample_ids)
        with tempfile.TemporaryDirectory() as temporary:
            regenerated = generate(FIXTURE_DIR / "blueprints.json")
            write_outputs(regenerated, Path(temporary), FIXTURE_DIR / "blueprints.json")
            assert (Path(temporary) / CORPUS_NAME).read_bytes() == args.corpus.read_bytes()
            regenerated_manifest = json.loads(
                (Path(temporary) / MANIFEST_NAME).read_text(encoding="utf-8")
            )
            assert regenerated_manifest == fixture_manifest
        if args.prediction is None:
            quality = summarize(review_manifest)
        else:
            quality = summarize(
                review_manifest,
                corpus_path=args.corpus,
                prediction_path=args.prediction,
                report_path=args.report,
            )
        print(
            json.dumps(
                {
                    "skills": len(EXPECTED_SKILLS),
                    "documents": len(records),
                    "corpusSha256": corpus_sha256,
                    "oracleRegression": oracle_regression,
                    "semanticFixture": semantic_fixture,
                    "quality": quality,
                },
                sort_keys=True,
            )
        )
    except (AssertionError, EvidenceError, OSError, json.JSONDecodeError, ValueError) as error:
        parser.error(f"verification failed: {error}")


if __name__ == "__main__":
    main()
