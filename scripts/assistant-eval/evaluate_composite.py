#!/usr/bin/env python3
"""Evaluate the Audio Host boundary smoke case for composed audio.

This is an audio-pipeline evaluator, not an Assistant semantic evaluator.  It
requires an explicit Audio Host prediction envelope and emits a separate
``qualityStatus: not_assessed`` field for Assistant semantics.  In particular,
``segments`` must not be confused with Assistant document ``children``.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


class EvaluationError(ValueError):
    """Raised when an artifact belongs to a different evaluation boundary."""


def _object(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise EvaluationError(f"{label} must be an object")
    return value


def validate_reference(reference: object) -> dict[str, object]:
    value = _object(reference, "reference")
    if value.get("schemaVersion") != 2:
        raise EvaluationError("reference schemaVersion must be 2")
    if value.get("type") != "compositeAudioBoundaryReference":
        raise EvaluationError("reference is not a composite audio boundary reference")
    if value.get("evaluationKind") != "audio_boundary_reference":
        raise EvaluationError("reference evaluationKind is not audio_boundary_reference")
    if value.get("sourceDomain") != "audio":
        raise EvaluationError("reference sourceDomain must be audio")
    if value.get("assistantSemanticQualityStatus") != "not_assessed":
        raise EvaluationError(
            "reference must declare Assistant semantic quality as not_assessed"
        )
    segments = value.get("segments")
    if not isinstance(segments, list) or len(segments) < 2:
        raise EvaluationError("reference segments must contain at least two clips")
    duration = value.get("durationMs")
    if not isinstance(duration, int) or duration <= 0:
        raise EvaluationError("reference durationMs must be a positive integer")
    _validate_intervals(segments, duration, "reference segments")
    return value


def validate_prediction(prediction: object) -> dict[str, object]:
    value = _object(prediction, "prediction")
    if value.get("predictionKind") == "assistant_semantic_prediction":
        raise EvaluationError(
            "Assistant semantic predictions cannot be scored by the audio evaluator"
        )
    if value.get("schemaVersion") != 2:
        raise EvaluationError("prediction schemaVersion must be 2")
    if value.get("predictionKind") != "audio_host_prediction":
        raise EvaluationError("prediction must declare predictionKind=audio_host_prediction")
    if value.get("sourceDomain") != "audio":
        raise EvaluationError("prediction sourceDomain must be audio")
    if value.get("assistantSemanticQualityStatus") != "not_assessed":
        raise EvaluationError(
            "prediction must declare Assistant semantic quality as not_assessed"
        )
    segments = value.get("segments")
    if not isinstance(segments, list):
        raise EvaluationError("Audio Host prediction segments must be an array")
    duration = value.get("durationMs")
    if not isinstance(duration, int) or duration <= 0:
        raise EvaluationError("prediction durationMs must be a positive integer")
    _validate_intervals(segments, duration, "prediction segments")
    return value


def _validate_intervals(
    segments: list[object], duration: int, label: str
) -> None:
    previous_end = 0
    for index, segment in enumerate(segments):
        item = _object(segment, f"{label}[{index}]")
        start = item.get("startMs")
        end = item.get("endMs")
        if not isinstance(start, int) or not isinstance(end, int):
            raise EvaluationError(f"{label}[{index}] interval must use integer milliseconds")
        if start < 0 or end <= start or end > duration:
            raise EvaluationError(f"{label}[{index}] interval is outside duration")
        if start < previous_end:
            raise EvaluationError(f"{label} intervals must be ordered and non-overlapping")
        previous_end = end


def boundaries(segments: list[dict[str, object]]) -> list[int]:
    return sorted(int(item["startMs"]) for item in segments if int(item["startMs"]) > 0)


def evaluate(reference: object, prediction: object, tolerance_ms: int = 2_000) -> dict[str, object]:
    reference_value = validate_reference(reference)
    prediction_value = validate_prediction(prediction)
    expected_segments = reference_value["segments"]
    predicted_segments = prediction_value["segments"]
    assert isinstance(expected_segments, list)
    assert isinstance(predicted_segments, list)
    expected = boundaries(expected_segments)
    predicted = boundaries(predicted_segments)
    matched = sum(
        any(abs(candidate - boundary) <= tolerance_ms for candidate in predicted)
        for boundary in expected
    )
    duration = reference_value["durationMs"]
    assert isinstance(duration, int)
    covered = sum(
        max(0, min(int(item["endMs"]), duration) - int(item["startMs"]))
        for item in predicted_segments
        if isinstance(item, dict)
    )
    return {
        "schemaVersion": 2,
        "evaluationKind": "audio_boundary_smoke",
        "sourceDomain": "audio",
        "predictionKind": "audio_host_prediction",
        "expectedBoundaries": expected,
        "predictedBoundaries": predicted,
        "boundaryRecall": matched / len(expected) if expected else 1.0,
        "coverageMs": covered,
        "durationMs": duration,
        "toleranceMs": tolerance_ms,
        "status": "audio_boundary_scored",
        "qualityStatus": "not_assessed",
        "qualityReason": (
            "This evaluator scores Audio Host boundary coverage only; it does not "
            "assess Assistant topic decisions, titles, summaries, or semantic quality."
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--prediction", type=Path, required=True)
    parser.add_argument("--tolerance-ms", type=int, default=2_000)
    args = parser.parse_args()
    reference = json.loads(args.reference.read_text(encoding="utf-8"))
    prediction = json.loads(args.prediction.read_text(encoding="utf-8"))
    report = evaluate(reference, prediction, args.tolerance_ms)
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
