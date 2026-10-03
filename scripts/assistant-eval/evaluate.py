#!/usr/bin/env python3
"""Offline semantic-boundary and structural evaluator for Assistant outputs."""

from __future__ import annotations

import argparse
import json
import math
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable


@dataclass(frozen=True)
class SegmentationScores:
    boundary_precision: float
    boundary_recall: float
    boundary_f1: float
    pk: float
    window_diff: float


@dataclass(frozen=True)
class StructureValidation:
    valid: bool
    duplicate_free: bool
    evidence_valid: bool
    hierarchy_valid: bool
    coverage: float
    gap_report_valid: bool
    errors: tuple[str, ...]


def boundary_set(labels: list[int]) -> set[int]:
    """Return boundary positions between items, represented by the right index."""
    return {
        index for index in range(1, len(labels)) if labels[index] != labels[index - 1]
    }


def boundary_prf(
    reference: list[int], hypothesis: list[int]
) -> tuple[float, float, float]:
    ref = boundary_set(reference)
    hyp = boundary_set(hypothesis)
    true_positive = len(ref & hyp)
    precision = true_positive / len(hyp) if hyp else (1.0 if not ref else 0.0)
    recall = true_positive / len(ref) if ref else (1.0 if not hyp else 0.0)
    f1 = 2 * precision * recall / (precision + recall) if precision + recall else 0.0
    return precision, recall, f1


def default_window(reference: list[int]) -> int:
    if len(reference) <= 1:
        return 1
    segment_count = len(set(reference))
    average_segment_length = len(reference) / max(1, segment_count)
    return max(1, min(len(reference) - 1, round(average_segment_length / 2)))


def pk(reference: list[int], hypothesis: list[int], window: int | None = None) -> float:
    if len(reference) != len(hypothesis):
        raise ValueError("reference and hypothesis lengths differ")
    if len(reference) <= 1:
        return 0.0
    k = default_window(reference) if window is None else window
    if not 1 <= k < len(reference):
        raise ValueError("Pk window must be within the sequence")
    errors = 0
    comparisons = len(reference) - k
    for index in range(comparisons):
        ref_same = reference[index] == reference[index + k]
        hyp_same = hypothesis[index] == hypothesis[index + k]
        errors += ref_same != hyp_same
    return errors / comparisons


def window_diff(
    reference: list[int], hypothesis: list[int], window: int | None = None
) -> float:
    if len(reference) != len(hypothesis):
        raise ValueError("reference and hypothesis lengths differ")
    if len(reference) <= 1:
        return 0.0
    k = default_window(reference) if window is None else window
    if not 1 <= k < len(reference):
        raise ValueError("WindowDiff window must be within the sequence")
    ref_boundaries = boundary_set(reference)
    hyp_boundaries = boundary_set(hypothesis)
    errors = 0
    comparisons = len(reference) - k
    for index in range(comparisons):
        window_range = range(index + 1, index + k + 1)
        ref_count = sum(position in ref_boundaries for position in window_range)
        hyp_count = sum(position in hyp_boundaries for position in window_range)
        errors += ref_count != hyp_count
    return errors / comparisons


def scores(reference: list[int], hypothesis: list[int]) -> SegmentationScores:
    precision, recall, f1 = boundary_prf(reference, hypothesis)
    return SegmentationScores(
        precision,
        recall,
        f1,
        pk(reference, hypothesis),
        window_diff(reference, hypothesis),
    )


def labels_from_children(
    segment_ids: list[str], children: list[dict[str, Any]]
) -> list[int]:
    index_by_id = {segment_id: index for index, segment_id in enumerate(segment_ids)}
    labels = [-1] * len(segment_ids)
    for child_index, child in enumerate(children):
        try:
            start = index_by_id[child["start_segment_id"]]
            end = index_by_id[child["end_segment_id"]]
        except KeyError as error:
            raise ValueError(f"unknown boundary segment: {error.args[0]}") from error
        if start > end:
            raise ValueError("child range is reversed")
        for index in range(start, end + 1):
            if labels[index] != -1:
                raise ValueError("child ranges overlap")
            labels[index] = child_index * 2

    next_gap_label = 1
    for index, value in enumerate(labels):
        if value != -1:
            continue
        if index > 0 and labels[index - 1] % 2 == 1:
            labels[index] = labels[index - 1]
        else:
            labels[index] = next_gap_label
            next_gap_label += 2
    return labels


def ranges(children: list[dict[str, Any]]) -> list[tuple[str, str]]:
    return [(child["start_segment_id"], child["end_segment_id"]) for child in children]


def _range_indices(
    values: list[dict[str, Any]],
    index_by_id: dict[str, int],
    label: str,
    errors: list[str],
) -> tuple[list[tuple[int, int] | None], set[int], bool]:
    ranges_found: list[tuple[int, int] | None] = []
    covered: set[int] = set()
    duplicate_free = True
    previous_end = -1
    for item_index, value in enumerate(values):
        if not isinstance(value, dict):
            errors.append(f"{label}[{item_index}] is not an object")
            ranges_found.append(None)
            continue
        start_id = value.get("start_segment_id")
        end_id = value.get("end_segment_id")
        if start_id not in index_by_id or end_id not in index_by_id:
            errors.append(f"{label}[{item_index}] references an unknown segment")
            ranges_found.append(None)
            continue
        start = index_by_id[start_id]
        end = index_by_id[end_id]
        if start > end:
            errors.append(f"{label}[{item_index}] is reversed")
            ranges_found.append(None)
            continue
        if start <= previous_end:
            errors.append(f"{label} is out of order or overlaps")
        previous_end = max(previous_end, end)
        item_indices = set(range(start, end + 1))
        if covered & item_indices:
            duplicate_free = False
            errors.append(f"{label} overlaps")
        covered.update(item_indices)
        ranges_found.append((start, end))
    return ranges_found, covered, duplicate_free


def validate_structure(
    segment_ids: list[str],
    mode: Any,
    children: Any,
    uncovered_ranges: Any,
    second_pass: Any = None,
    topic_hierarchy: Any = None,
) -> StructureValidation:
    errors: list[str] = []
    if not segment_ids or len(segment_ids) != len(set(segment_ids)):
        errors.append("segment IDs must be non-empty and unique")
    index_by_id = {segment_id: index for index, segment_id in enumerate(segment_ids)}
    all_indices = set(range(len(segment_ids)))
    if mode not in {"partition", "extract", "none"}:
        errors.append("mode is not partition, extract, or none")
    if not isinstance(children, list):
        errors.append("children is not an array")
        children = []
    if not isinstance(uncovered_ranges, list):
        errors.append("uncovered_ranges is not an array")
        uncovered_ranges = []

    child_ranges, covered, duplicate_free = _range_indices(
        children, index_by_id, "children", errors
    )
    evidence_valid = True
    for child_index, (child, child_range) in enumerate(zip(children, child_ranges)):
        if not isinstance(child, dict) or child_range is None:
            evidence_valid = False
            continue
        if "children" in child or "second_pass" in child:
            errors.append(
                f"children[{child_index}] creates more than one level in one call"
            )
        evidence = child.get("evidence_segment_ids")
        start, end = child_range
        if (
            not isinstance(evidence, list)
            or not evidence
            or len(evidence) != len(set(evidence))
            or any(
                item not in index_by_id or not start <= index_by_id[item] <= end
                for item in evidence
            )
        ):
            evidence_valid = False
            errors.append(f"children[{child_index}] has invalid evidence")

    _, uncovered, uncovered_duplicate_free = _range_indices(
        uncovered_ranges, index_by_id, "uncovered_ranges", errors
    )
    duplicate_free = (
        duplicate_free and uncovered_duplicate_free and not bool(covered & uncovered)
    )
    if covered & uncovered:
        errors.append("children and uncovered_ranges overlap")
    complement = all_indices - covered
    gap_report_valid = uncovered == complement
    if mode == "partition":
        if covered != all_indices:
            errors.append("partition does not cover every segment")
        if uncovered:
            errors.append("partition reports uncovered segments")
        gap_report_valid = covered == all_indices and not uncovered
    elif mode == "extract":
        if not gap_report_valid:
            errors.append("extract uncovered_ranges is not the exact complement")
    elif mode == "none":
        if children or uncovered_ranges:
            errors.append("none mode must not contain children or uncovered ranges")
        gap_report_valid = not children and not uncovered_ranges

    hierarchy_valid = True
    if second_pass is not None:
        if not isinstance(second_pass, dict):
            hierarchy_valid = False
            errors.append("second_pass is not an object")
        else:
            parent_index = second_pass.get("parent_child_index")
            if (
                not isinstance(parent_index, int)
                or not 0 <= parent_index < len(child_ranges)
                or child_ranges[parent_index] is None
            ):
                hierarchy_valid = False
                errors.append("second_pass parent_child_index is invalid")
            else:
                parent_start, parent_end = child_ranges[parent_index]
                parent_ids = segment_ids[parent_start : parent_end + 1]
                nested = validate_structure(
                    parent_ids,
                    second_pass.get("mode"),
                    second_pass.get("children"),
                    second_pass.get("uncovered_ranges", []),
                )
                if not nested.valid:
                    hierarchy_valid = False
                    errors.extend(f"second_pass: {error}" for error in nested.errors)

    if topic_hierarchy is not None:
        if not isinstance(topic_hierarchy, dict) or topic_hierarchy.get(
            "maxTreeDepth"
        ) not in {0, 1, 2}:
            hierarchy_valid = False
            errors.append("topic hierarchy exceeds the three-level document tree")
        else:
            nested_topics = topic_hierarchy.get("nestedTopics", [])
            if not isinstance(nested_topics, list):
                hierarchy_valid = False
                errors.append("nestedTopics is not an array")
            else:
                for topic_index, topic in enumerate(nested_topics):
                    if not isinstance(topic, dict) or topic.get("depth") != 2:
                        hierarchy_valid = False
                        errors.append(f"nestedTopics[{topic_index}] has invalid depth")
                        continue
                    topic_errors: list[str] = []
                    topic_ranges, _, _ = _range_indices(
                        [topic], index_by_id, "nestedTopics", topic_errors
                    )
                    if topic_errors or not topic_ranges or topic_ranges[0] is None:
                        hierarchy_valid = False
                        errors.extend(topic_errors)
                        continue
                    evidence = topic.get("evidence_segment_ids")
                    start, end = topic_ranges[0]
                    if (
                        not isinstance(evidence, list)
                        or not evidence
                        or any(
                            item not in index_by_id
                            or not start <= index_by_id[item] <= end
                            for item in evidence
                        )
                    ):
                        hierarchy_valid = False
                        evidence_valid = False
                        errors.append(
                            f"nestedTopics[{topic_index}] has invalid evidence"
                        )
                    parent_topic_id = topic.get("parentTopicId")
                    parent_ranges = [
                        child_range
                        for child, child_range in zip(children, child_ranges)
                        if child_range is not None
                        and isinstance(child, dict)
                        and child.get("sourceRef", {}).get("topicId") == parent_topic_id
                    ]
                    if not any(
                        parent_start <= start <= end <= parent_end
                        for parent_start, parent_end in parent_ranges
                    ):
                        hierarchy_valid = False
                        errors.append(
                            f"nestedTopics[{topic_index}] is outside its parent topic"
                        )

    valid = (
        not errors
        and duplicate_free
        and evidence_valid
        and hierarchy_valid
        and gap_report_valid
    )
    return StructureValidation(
        valid=valid,
        duplicate_free=duplicate_free,
        evidence_valid=evidence_valid,
        hierarchy_valid=hierarchy_valid,
        coverage=len(covered) / len(segment_ids) if segment_ids else 1.0,
        gap_report_valid=gap_report_valid,
        errors=tuple(errors),
    )


def equal_length_labels(length: int, segment_count: int) -> list[int]:
    if length == 0:
        return []
    segment_count = max(1, min(length, segment_count))
    return [
        min(segment_count - 1, math.floor(index * segment_count / length))
        for index in range(length)
    ]


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    records = []
    with path.open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, 1):
            if not line.strip():
                continue
            try:
                records.append(json.loads(line))
            except json.JSONDecodeError as error:
                raise ValueError(
                    f"{path}:{line_number}: invalid JSON: {error}"
                ) from error
    return records


def segment_ids_for(record: dict[str, Any]) -> list[str]:
    if "segments" in record:
        return [segment["id"] for segment in record["segments"]]
    return [
        segment["id"] for segment in record["record"]["state"]["results"]["transcript"]
    ]


def validate_gold_record(record: dict[str, Any]) -> StructureValidation:
    segment_ids = segment_ids_for(record)
    gold = record.get("gold", {})
    validation = validate_structure(
        segment_ids,
        gold.get("allowed_mode"),
        gold.get("children"),
        gold.get("uncovered_ranges", []),
        gold.get("second_pass"),
        record.get("topicHierarchy"),
    )
    extra_errors = list(validation.errors)
    segments = record.get("segments") or record.get("record", {}).get("state", {}).get(
        "results", {}
    ).get("transcript", [])
    previous_start: int | None = None
    for segment_index, segment in enumerate(segments):
        start = segment.get("startMs", segment.get("start_ms"))
        end = segment.get("endMs", segment.get("end_ms"))
        if not isinstance(start, int) or not isinstance(end, int) or start >= end:
            extra_errors.append(f"segment[{segment_index}] has invalid timing")
            continue
        if previous_start is not None and start < previous_start:
            extra_errors.append(f"segment[{segment_index}] start time goes backwards")
        previous_start = start
    for child_index, child in enumerate(gold.get("children", [])):
        if not isinstance(child, dict):
            continue
        titles = child.get("title")
        tags = child.get("tags")
        summaries = child.get("summary")
        if (
            not isinstance(titles, list)
            or not titles
            or not all(isinstance(value, str) and value for value in titles)
        ):
            extra_errors.append(
                f"gold.children[{child_index}] has invalid accepted titles"
            )
        if (
            not isinstance(tags, list)
            or not tags
            or not all(
                isinstance(option, list)
                and option
                and all(isinstance(value, str) and value for value in option)
                for option in tags
            )
        ):
            extra_errors.append(
                f"gold.children[{child_index}] has invalid accepted tags"
            )
        if not isinstance(summaries, list) or not all(
            isinstance(value, str) and value for value in summaries
        ):
            extra_errors.append(
                f"gold.children[{child_index}] has invalid accepted summaries"
            )
    if "record" in record and "expected_parent_revision" in gold:
        revision = record["record"]["state"]["schema"]["revision"]
        if gold["expected_parent_revision"] != revision:
            extra_errors.append("expected_parent_revision does not match the document")
    if "forbidden_operations" in gold:
        forbidden = gold["forbidden_operations"]
        if (
            not isinstance(forbidden, list)
            or not forbidden
            or len(forbidden) != len(set(forbidden))
        ):
            extra_errors.append("forbidden_operations must be a non-empty unique array")
    if extra_errors != list(validation.errors):
        validation = StructureValidation(
            valid=False,
            duplicate_free=validation.duplicate_free,
            evidence_valid=validation.evidence_valid,
            hierarchy_valid=validation.hierarchy_valid,
            coverage=validation.coverage,
            gap_report_valid=validation.gap_report_valid,
            errors=tuple(extra_errors),
        )
    return validation


def evaluate_records(
    gold_records: Iterable[dict[str, Any]], predictions: Iterable[dict[str, Any]]
) -> dict[str, Any]:
    gold_list = list(gold_records)
    prediction_list = list(predictions)
    gold_by_id = {record["scenarioId"]: record for record in gold_list}
    prediction_by_id = {record["scenarioId"]: record for record in prediction_list}
    if len(gold_by_id) != len(gold_list):
        raise ValueError("gold contains duplicate scenarioId values")
    if len(prediction_by_id) != len(prediction_list):
        raise ValueError("predictions contain duplicate scenarioId values")
    invalid_gold = {
        scenario_id: list(validation.errors)
        for scenario_id, record in gold_by_id.items()
        if not (validation := validate_gold_record(record)).valid
    }
    if invalid_gold:
        raise ValueError(
            f"invalid gold records: {json.dumps(invalid_gold, sort_keys=True)}"
        )
    missing = sorted(set(gold_by_id) - set(prediction_by_id))
    unexpected = sorted(set(prediction_by_id) - set(gold_by_id))
    examples: list[dict[str, Any]] = []
    for scenario_id in sorted(gold_by_id):
        gold_record = gold_by_id[scenario_id]
        prediction = prediction_by_id.get(scenario_id, {})
        segment_ids = segment_ids_for(gold_record)
        reference_children = gold_record["gold"]["children"]
        predicted_children = prediction.get("children", [])
        try:
            reference_labels = labels_from_children(segment_ids, reference_children)
            predicted_labels = labels_from_children(segment_ids, predicted_children)
            measured = scores(reference_labels, predicted_labels)
        except (KeyError, TypeError, ValueError) as error:
            measured = SegmentationScores(0.0, 0.0, 0.0, 1.0, 1.0)
            structural_error = str(error)
        else:
            structural_error = None
        validation = validate_structure(
            segment_ids,
            prediction.get("mode"),
            predicted_children,
            prediction.get("uncovered_ranges", []),
            prediction.get("second_pass"),
        )
        if validation.errors:
            structural_error = "; ".join(validation.errors)
        try:
            exact = (
                prediction.get("mode") == gold_record["gold"]["allowed_mode"]
                and ranges(predicted_children) == ranges(reference_children)
                and [
                    child.get("evidence_segment_ids", [])
                    for child in predicted_children
                ]
                == [
                    child.get("evidence_segment_ids", [])
                    for child in reference_children
                ]
                and prediction.get("uncovered_ranges", [])
                == gold_record["gold"].get("uncovered_ranges", [])
                and prediction.get("second_pass")
                == gold_record["gold"].get("second_pass")
            )
        except (KeyError, TypeError):
            exact = False
        example = {
            "scenarioId": scenario_id,
            "referenceMode": gold_record["gold"]["allowed_mode"],
            "exact": exact,
            "structurallyValid": validation.valid,
            "duplicateFree": validation.duplicate_free,
            "evidenceValid": validation.evidence_valid,
            "hierarchyValid": validation.hierarchy_valid,
            "gapReportValid": validation.gap_report_valid,
            "coverage": validation.coverage,
            **measured.__dict__,
        }
        if structural_error:
            example["error"] = structural_error
        examples.append(example)

    def mean(field: str) -> float:
        return (
            sum(float(example[field]) for example in examples) / len(examples)
            if examples
            else 0.0
        )

    def mean_for_mode(field: str, mode: str) -> float:
        selected = [example for example in examples if example["referenceMode"] == mode]
        return (
            sum(float(example[field]) for example in selected) / len(selected)
            if selected
            else 1.0
        )

    return {
        "schemaVersion": 1,
        "exampleCount": len(examples),
        "missingScenarioIds": missing,
        "unexpectedScenarioIds": unexpected,
        "exactMatchRate": mean("exact"),
        "structuralValidityRate": mean("structurallyValid"),
        "duplicateFreeRate": mean("duplicateFree"),
        "evidenceValidityRate": mean("evidenceValid"),
        "hierarchyValidityRate": mean("hierarchyValid"),
        "gapReportValidityRate": mean("gapReportValid"),
        "meanCoverage": mean("coverage"),
        "partitionCoverageRate": mean_for_mode("coverage", "partition"),
        "extractGapReportValidityRate": mean_for_mode("gapReportValid", "extract"),
        "boundaryPrecision": mean("boundary_precision"),
        "boundaryRecall": mean("boundary_recall"),
        "boundaryF1": mean("boundary_f1"),
        "Pk": mean("pk"),
        "WindowDiff": mean("window_diff"),
        "goldValidation": "passed",
        "examples": examples,
    }


def baseline_predictions(
    records: Iterable[dict[str, Any]], kind: str
) -> list[dict[str, Any]]:
    predictions = []
    for record in records:
        segment_ids = segment_ids_for(record)
        reference_children = record["gold"]["children"]
        if not reference_children:
            child_ranges = []
        elif kind == "no-split":
            child_ranges = [(0, len(segment_ids) - 1)] if segment_ids else []
        elif kind == "equal-length":
            count = len(set(labels_from_children(segment_ids, reference_children)))
            labels = equal_length_labels(len(segment_ids), count)
            child_ranges = []
            start = 0
            for index in range(1, len(labels) + 1):
                if index == len(labels) or labels[index] != labels[start]:
                    child_ranges.append((start, index - 1))
                    start = index
        else:
            raise ValueError(f"unknown baseline: {kind}")
        children = [
            {
                "start_segment_id": segment_ids[start],
                "end_segment_id": segment_ids[end],
                "evidence_segment_ids": [segment_ids[start]],
            }
            for start, end in child_ranges
        ]
        predictions.append(
            {
                "scenarioId": record["scenarioId"],
                "mode": record["gold"]["allowed_mode"],
                "children": children,
                "uncovered_ranges": [],
            }
        )
    return predictions


def oracle_predictions(records: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            "scenarioId": record["scenarioId"],
            "mode": record["gold"]["allowed_mode"],
            "children": record["gold"]["children"],
            "uncovered_ranges": record["gold"].get("uncovered_ranges", []),
            **(
                {"second_pass": record["gold"]["second_pass"]}
                if "second_pass" in record["gold"]
                else {}
            ),
        }
        for record in records
    ]


def compare_with_baselines(
    records: list[dict[str, Any]], predictions: list[dict[str, Any]]
) -> dict[str, Any]:
    candidate = evaluate_records(records, predictions)
    no_split = evaluate_records(records, baseline_predictions(records, "no-split"))
    equal_length = evaluate_records(
        records, baseline_predictions(records, "equal-length")
    )

    def better_than(baseline: dict[str, Any]) -> bool:
        return (
            candidate["boundaryF1"] > baseline["boundaryF1"]
            and candidate["Pk"] < baseline["Pk"]
            and candidate["WindowDiff"] < baseline["WindowDiff"]
        )

    return {
        "schemaVersion": 1,
        "candidate": candidate,
        "baselines": {"noSplit": no_split, "equalLength": equal_length},
        "outperforms": {
            "noSplitOnAllMetrics": better_than(no_split),
            "equalLengthOnAllMetrics": better_than(equal_length),
        },
    }


def select_split(records: Iterable[dict[str, Any]], split: str) -> list[dict[str, Any]]:
    records = list(records)
    if split == "all":
        return records
    if split == "smoke":
        return [record for record in records if record.get("smoke") is True]
    if split == "full":
        return [record for record in records if record.get("smoke") is not True]
    raise ValueError(f"unknown evaluation split: {split}")


def validate_run_metadata(metadata: Any) -> dict[str, Any]:
    """Validate auditable run metadata without permitting credential material."""
    if not isinstance(metadata, dict):
        raise ValueError("run metadata must be a JSON object")
    forbidden = {"secret", "token", "apiKey", "api_key", "password", "credential"}
    leaked: list[str] = []

    def find_forbidden(value: Any, path: str = "") -> None:
        if isinstance(value, dict):
            for key, child in value.items():
                child_path = f"{path}.{key}" if path else str(key)
                if key in forbidden:
                    leaked.append(child_path)
                find_forbidden(child, child_path)
        elif isinstance(value, list):
            for index, child in enumerate(value):
                find_forbidden(child, f"{path}[{index}]")

    find_forbidden(metadata)
    if leaked:
        raise ValueError(f"run metadata contains forbidden secret fields: {leaked}")
    required = {"model", "prompt", "budget", "elapsedMs", "resources"}
    missing = sorted(required - set(metadata))
    if missing:
        raise ValueError(f"run metadata is missing fields: {missing}")
    if not isinstance(metadata["model"], str) or not metadata["model"]:
        raise ValueError("run metadata model must be a non-empty string")
    if not isinstance(metadata["prompt"], str) or not metadata["prompt"]:
        raise ValueError("run metadata prompt must be a non-empty string")
    if not isinstance(metadata["budget"], dict) or not isinstance(metadata["resources"], dict):
        raise ValueError("run metadata budget and resources must be objects")
    if not isinstance(metadata["elapsedMs"], int) or isinstance(metadata["elapsedMs"], bool) or metadata["elapsedMs"] < 0:
        raise ValueError("run metadata elapsedMs must be a non-negative integer")
    return metadata


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--gold", type=Path, required=True)
    parser.add_argument("--predictions", type=Path)
    parser.add_argument("--baseline", choices=("no-split", "equal-length"))
    parser.add_argument("--split", choices=("all", "smoke", "full"), default="all")
    parser.add_argument(
        "--metadata",
        type=Path,
        help="JSON run metadata: model, prompt, budget, elapsedMs, resources; never secrets",
    )
    parser.add_argument(
        "--oracle",
        action="store_true",
        help="Copy gold labels to self-test the evaluator",
    )
    parser.add_argument("--compare-baselines", action="store_true")
    parser.add_argument("--summary-only", action="store_true")
    args = parser.parse_args()
    if sum((bool(args.predictions), bool(args.baseline), args.oracle)) != 1:
        parser.error("choose exactly one of --predictions, --baseline, or --oracle")
    if args.compare_baselines and args.baseline:
        parser.error("--compare-baselines requires predictions or --oracle")
    gold = select_split(read_jsonl(args.gold), args.split)
    if args.predictions:
        expected_ids = {record["scenarioId"] for record in gold}
        predictions = [
            prediction
            for prediction in read_jsonl(args.predictions)
            if prediction.get("scenarioId") in expected_ids
        ]
        prediction_source = "external"
    elif args.oracle:
        predictions = oracle_predictions(gold)
        prediction_source = "oracle-gold-copy"
    else:
        predictions = baseline_predictions(gold, args.baseline)
        prediction_source = f"baseline:{args.baseline}"
    report = (
        compare_with_baselines(gold, predictions)
        if args.compare_baselines
        else evaluate_records(gold, predictions)
    )
    report["predictionSource"] = prediction_source
    report["split"] = args.split
    if args.metadata:
        report["runMetadata"] = validate_run_metadata(
            json.loads(args.metadata.read_text(encoding="utf-8"))
        )
    if args.summary_only:
        if args.compare_baselines:
            report["candidate"].pop("examples", None)
            for baseline in report["baselines"].values():
                baseline.pop("examples", None)
        else:
            report.pop("examples", None)
    print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
