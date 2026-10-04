#!/usr/bin/env python3
"""Score canonical ASR/diarization predictions against dataset references.

The prediction format intentionally mirrors the reference projection while
keeping model output separate.  This script is offline and deterministic; it
does not download data or call a model.
"""

from __future__ import annotations

import argparse
import difflib
import itertools
import json
import re
from collections import defaultdict
from pathlib import Path
from typing import Any, Iterable


TOKEN = re.compile(r"\w+|[^\w\s]", re.UNICODE)


class EvaluationError(ValueError):
    """Raised when a prediction crosses the Audio Host evaluation boundary."""


def validate_audio_host_prediction(prediction: object) -> dict[str, Any]:
    if not isinstance(prediction, dict):
        raise EvaluationError("prediction must be an object")
    if prediction.get("predictionKind") == "assistant_semantic_prediction":
        raise EvaluationError(
            "Assistant semantic predictions cannot be scored by audio metrics"
        )
    if prediction.get("schemaVersion") != 2:
        raise EvaluationError("prediction schemaVersion must be 2")
    if prediction.get("predictionKind") != "audio_host_prediction":
        raise EvaluationError("prediction must declare predictionKind=audio_host_prediction")
    if prediction.get("sourceDomain") != "audio":
        raise EvaluationError("prediction sourceDomain must be audio")
    if prediction.get("assistantSemanticQualityStatus") != "not_assessed":
        raise EvaluationError(
            "prediction must declare Assistant semantic quality as not_assessed"
        )
    return prediction


def tokens(text: str, language: str) -> list[str]:
    if language.startswith("zh"):
        return [char for char in text if not char.isspace()]
    return TOKEN.findall(text.lower())


def edit_distance(left: list[str], right: list[str]) -> int:
    if left == right:
        return 0
    matcher = difflib.SequenceMatcher(a=left, b=right, autojunk=True)
    return sum(
        max(i2 - i1, j2 - j1)
        for tag, i1, i2, j1, j2 in matcher.get_opcodes()
        if tag != "equal"
    )


def edit_distance_with_method(left: list[str], right: list[str]) -> tuple[int, str]:
    if len(left) * len(right) <= 10_000_000:
        row = list(range(len(right) + 1))
        for i, source in enumerate(left, 1):
            next_row = [i]
            for j, target in enumerate(right, 1):
                next_row.append(
                    min(
                        next_row[-1] + 1,
                        row[j] + 1,
                        row[j - 1] + (source != target),
                    )
                )
            row = next_row
        return row[-1], "levenshtein"
    return edit_distance(left, right), "sequence_matcher_approximation"


def interval_sets(
    segments: Iterable[dict[str, Any]], boundaries: list[int], collar_ms: int = 0
) -> list[set[str]]:
    events: dict[int, tuple[set[str], set[str]]] = defaultdict(lambda: (set(), set()))
    for segment in segments:
        start = max(0, int(segment["startMs"]) - collar_ms)
        end = int(segment["endMs"]) + collar_ms
        if end <= start:
            continue
        events[start][0].add(str(segment["speaker"]))
        events[end][1].add(str(segment["speaker"]))
    active: set[str] = set()
    result: list[set[str]] = []
    for start, end in zip(boundaries, boundaries[1:]):
        additions, removals = events.get(start, (set(), set()))
        active.difference_update(removals)
        active.update(additions)
        result.append(set(active) if end > start else set())
    return result


def diarization(
    reference: list[dict[str, Any]],
    prediction: list[dict[str, Any]],
    collar_ms: int = 0,
) -> dict[str, float]:
    boundaries = {0}
    for segment in (*reference, *prediction):
        boundaries.add(max(0, int(segment["startMs"]) - collar_ms))
        boundaries.add(int(segment["endMs"]) + collar_ms)
    points = sorted(boundaries)
    ref_speakers = sorted({str(item["speaker"]) for item in reference})
    pred_speakers = sorted({str(item["speaker"]) for item in prediction})
    ref_sets = interval_sets(reference, points, collar_ms)
    pred_sets = interval_sets(prediction, points, collar_ms)
    best = None
    for mapped in itertools.permutations(
        ref_speakers, min(len(ref_speakers), len(pred_speakers))
    ):
        mapping = dict(zip(pred_speakers, mapped))
        miss = false_alarm = confusion = speech = 0
        for (start, end), ref, raw_hyp in zip(
            zip(points, points[1:]), ref_sets, pred_sets
        ):
            if end <= start:
                continue
            duration = end - start
            hyp = {mapping.get(speaker, speaker) for speaker in raw_hyp}
            speech += duration * len(ref)
            miss += duration * len(ref - hyp)
            false_alarm += duration * len(hyp - ref)
            confusion += duration * min(len(ref & hyp), len(ref - hyp), len(hyp - ref))
        candidate = (miss, false_alarm, confusion, speech)
        if best is None or (miss + false_alarm + confusion) < sum(best[:3]):
            best = candidate
    miss, false_alarm, confusion, speech = best or (0, 0, 0, 0)
    denominator = max(1, speech)
    return {
        "der": (miss + false_alarm + confusion) / denominator,
        "missRate": miss / denominator,
        "falseAlarmRate": false_alarm / denominator,
        "confusionRate": confusion / denominator,
    }


def score_record(
    reference: dict[str, Any], prediction: dict[str, Any], collar_ms: int = 0
) -> dict[str, Any]:
    validate_audio_host_prediction(prediction)
    if "referenceSpeakerSegments" in prediction:
        raise EvaluationError(
            "prediction must not contain referenceSpeakerSegments; gold annotations "
            "cannot be used as Audio Host output"
        )
    language = str(reference.get("language", "en"))
    ref_text = tokens(
        " ".join(str(item.get("text", "")) for item in reference["transcript"]),
        language,
    )
    pred_text = tokens(
        " ".join(str(item.get("text", "")) for item in prediction.get("transcript", [])),
        language,
    )
    errors, method = edit_distance_with_method(ref_text, pred_text)
    asr = {
        "errors": errors,
        "referenceTokens": len(ref_text),
        "werOrCer": errors / max(1, len(ref_text)),
        "method": method,
    }
    diar = diarization(
        reference["referenceSpeakerSegments"],
        prediction.get("speakerSegments", []),
        collar_ms,
    )
    return {
        "meetingId": reference["meetingId"],
        "evaluationKind": "audio_host_quality",
        "asr": asr,
        "diarization": diar,
        "assistantSemanticQualityStatus": "not_assessed",
        "assistantSemanticQualityReason": (
            "ASR and diarization metrics do not evaluate Assistant semantic "
            "segmentation, titles, summaries, or document-tree decisions."
        ),
    }


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--predictions", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--collar-ms", type=int, default=0)
    args = parser.parse_args()
    references = {record["meetingId"]: record for record in read_jsonl(args.reference)}
    predictions = {record["meetingId"]: record for record in read_jsonl(args.predictions)}
    missing = sorted(set(references) - set(predictions))
    if missing:
        raise SystemExit(f"missing predictions for meetings: {missing}")
    rows = [
        score_record(references[key], predictions[key], args.collar_ms)
        for key in sorted(references)
    ]
    report = {
        "schemaVersion": 2,
        "evaluationKind": "audio_host_quality",
        "records": rows,
        "status": "audio_metrics_scored",
        "qualityStatus": "not_assessed",
        "qualityReason": (
            "No Assistant semantic quality is assessed by Audio Host ASR/diarization "
            "metrics."
        ),
    }
    encoded = json.dumps(report, ensure_ascii=False, indent=2)
    if args.output:
        args.output.write_text(encoded + "\n", encoding="utf-8")
    print(encoded)


if __name__ == "__main__":
    main()
