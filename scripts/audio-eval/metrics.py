#!/usr/bin/env python3
"""Score canonical ASR/diarization predictions against dataset references.

The prediction format intentionally mirrors the reference projection while
keeping model output separate.  This script is offline and deterministic; it
does not download data or call a model.
"""

from __future__ import annotations

import argparse
import difflib
import importlib.metadata
import math
import hashlib
import os
import tempfile
import json
import re
from pathlib import Path
from typing import Any


TOKEN = re.compile(r"\w+|[^\w\s]", re.UNICODE)


class EvaluationError(ValueError):
    """Raised when a prediction crosses the Audio Host evaluation boundary."""


def validate_audio_host_prediction(prediction: object) -> dict[str, Any]:
    if not isinstance(prediction, dict):
        raise EvaluationError("prediction must be an object")
    if prediction.get("predictionKind") == "assistant_semantic_prediction":
        raise EvaluationError("Assistant semantic predictions cannot be scored by audio metrics")
    if prediction.get("schemaVersion") != 2:
        raise EvaluationError("prediction schemaVersion must be 2")
    if prediction.get("predictionKind") != "audio_host_prediction":
        raise EvaluationError("prediction must declare predictionKind=audio_host_prediction")
    if prediction.get("sourceDomain") != "audio":
        raise EvaluationError("prediction sourceDomain must be audio")
    if prediction.get("assistantSemanticQualityStatus") != "not_assessed":
        raise EvaluationError("prediction must declare Assistant semantic quality as not_assessed")
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
        max(i2 - i1, j2 - j1) for tag, i1, i2, j1, j2 in matcher.get_opcodes() if tag != "equal"
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


class DiarizationUnavailable(EvaluationError):
    """The optional, pinned standard scorer is not installed."""


def diarization(
    reference: list[dict[str, Any]],
    prediction: list[dict[str, Any]],
    collar_ms: int = 0,
    duration_ms: int | None = None,
) -> dict[str, Any]:
    if type(collar_ms) is not int or collar_ms < 0:
        raise EvaluationError("collarMs must be a nonnegative integer (full collar duration)")
    if duration_ms is not None and (type(duration_ms) is not int or duration_ms <= 0):
        raise EvaluationError("durationMs must be a positive integer")
    try:
        finite = math.isfinite(collar_ms / 1000) and (
            duration_ms is None or math.isfinite(duration_ms / 1000)
        )
    except OverflowError:
        finite = False
    if not finite:
        raise EvaluationError("duration or collar exceeds scorer time precision")
    for source in (reference, prediction):
        for segment in source:
            start, end = segment.get("startMs"), segment.get("endMs")
            if (
                type(start) is not int
                or type(end) is not int
                or not 0 <= start < end
                or not isinstance(segment.get("speaker"), str)
                or not segment["speaker"].strip()
                or (duration_ms is not None and end > duration_ms)
            ):
                raise EvaluationError("invalid speaker interval, label or source duration")
            try:
                valid = math.isfinite(end / 1000) and start / 1000 < end / 1000
            except OverflowError:
                valid = False
            if not valid:
                raise EvaluationError("speaker interval exceeds scorer time precision")
    try:
        if importlib.metadata.version("pyannote.metrics") != "4.0.0":
            raise DiarizationUnavailable("requires pinned pyannote.metrics==4.0.0")
        from pyannote.core import Annotation, Segment, Timeline
        from pyannote.metrics.diarization import DiarizationErrorRate
    except (ImportError, importlib.metadata.PackageNotFoundError) as error:
        raise DiarizationUnavailable("install requirements-diarization.txt to score DER") from error

    def annotation(segments):
        result = Annotation(uri="recording")
        for index, segment in enumerate(segments):
            result[Segment(segment["startMs"] / 1000, segment["endMs"] / 1000), index] = segment[
                "speaker"
            ]
        # Multiple overlapping tracks of one label are one active speaker.
        return result.support()

    end_ms = (
        duration_ms
        if duration_ms is not None
        else max((segment["endMs"] for segment in reference + prediction), default=0)
    )
    region = Timeline([Segment(0, end_ms / 1000)]) if end_ms else Timeline()
    details = DiarizationErrorRate(collar=collar_ms / 1000, skip_overlap=False)(
        annotation(reference), annotation(prediction), uem=region, detailed=True
    )
    total = float(details["total"])
    scored = total > 0
    return {
        "status": "measured" if scored else "unscored_no_reference_speech",
        "method": "pyannote.metrics-4.0.0-hungarian",
        "collarMs": collar_ms,
        "collarConvention": "full_duration_centered_on_reference_boundaries",
        "overlap": "included",
        "scoringRegion": "source_duration" if duration_ms is not None else "observed_extent",
        "scoredSpeakerSeconds": total,
        "missedSpeakerSeconds": float(details["missed detection"]),
        "falseAlarmSpeakerSeconds": float(details["false alarm"]),
        "confusedSpeakerSeconds": float(details["confusion"]),
        "der": float(details["diarization error rate"]) if scored else None,
        "missRate": float(details["missed detection"]) / total if scored else None,
        "falseAlarmRate": float(details["false alarm"]) / total if scored else None,
        "confusionRate": float(details["confusion"]) / total if scored else None,
    }


def score_record(
    reference: dict[str, Any], prediction: dict[str, Any], collar_ms: int = 0
) -> dict[str, Any]:
    validate_audio_host_prediction(prediction)
    if "meetingId" in prediction and prediction["meetingId"] != reference.get("meetingId"):
        raise EvaluationError("prediction meetingId does not match its reference")
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
    if "speakerSegments" not in prediction:
        diar = {"status": "unavailable", "der": None, "reason": "prediction has no speakerSegments"}
    else:
        try:
            diar = diarization(
                reference["referenceSpeakerSegments"],
                prediction["speakerSegments"],
                collar_ms,
                reference.get("durationMs"),
            )
        except DiarizationUnavailable as error:
            diar = {"status": "unavailable", "der": None, "reason": str(error)}
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


def records_by_id(path: Path, payload: bytes) -> dict[str, dict[str, Any]]:
    result = {}
    for line in payload.decode("utf-8").splitlines():
        if not line.strip():
            continue
        record = json.loads(line)
        identity = record.get("meetingId") if isinstance(record, dict) else None
        if not isinstance(identity, str) or not identity.strip() or identity in result:
            raise EvaluationError(f"invalid or duplicate meetingId in {path}: {identity!r}")
        result[identity] = record
    if not result:
        raise EvaluationError(f"no meetings in {path}")
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--predictions", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--collar-ms", type=int, default=0)
    args = parser.parse_args()
    inputs = [args.reference, args.predictions]
    if args.output:
        if any(
            args.output.resolve() == path.resolve()
            or (args.output.exists() and path.exists() and args.output.samefile(path))
            for path in inputs
        ):
            raise EvaluationError("report output must not overwrite an input")
        # An unsuccessful rerun must not leave an older successful receipt.
        args.output.unlink(missing_ok=True)
    # Parse and hash the same immutable snapshots, even if an external writer
    # replaces an input while scoring is in progress.
    payloads = {path: path.read_bytes() for path in inputs}
    references = records_by_id(args.reference, payloads[args.reference])
    predictions = records_by_id(args.predictions, payloads[args.predictions])
    unexpected = sorted(set(predictions) - set(references))
    if unexpected:
        raise EvaluationError(f"unexpected predictions for meetings: {unexpected}")
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
        "referenceSha256": hashlib.sha256(payloads[args.reference]).hexdigest(),
        "predictionSha256": hashlib.sha256(payloads[args.predictions]).hexdigest(),
        "scorerSha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "records": rows,
        "status": (
            "audio_metrics_scored"
            if all(row["diarization"]["status"] == "measured" for row in rows)
            else "audio_metrics_partial"
        ),
        "qualityStatus": "not_assessed",
        "qualityReason": (
            "No Assistant semantic quality is assessed by Audio Host ASR/diarization " "metrics."
        ),
    }
    encoded = json.dumps(report, ensure_ascii=False, indent=2)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(
                mode="w",
                encoding="utf-8",
                dir=args.output.parent,
                prefix=".audio-metrics-",
                delete=False,
            ) as stream:
                temporary = Path(stream.name)
                stream.write(encoded + "\n")
            os.replace(temporary, args.output)
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
    print(encoded)


if __name__ == "__main__":
    main()
