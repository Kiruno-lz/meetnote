#!/usr/bin/env python3
"""Experimental MLX comparison only; never a desktop runtime or DIA provider."""
import json
import os
import sys
import time
from pathlib import Path

# Models must already be pinned and downloaded by the experiment owner.
os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"


def main():
    import mlx.core as mx
    from mlx_qwen3_asr import Session

    config_path, cases_path, output_path = map(Path, sys.argv[1:])
    config = json.loads(config_path.read_text())
    cases = json.loads(cases_path.read_text())
    if config["device"] != "Metal" or not mx.metal.is_available():
        raise RuntimeError("this experimental runner requires an available Metal device")
    mx.set_default_device(mx.gpu)
    started = time.monotonic()
    session = Session(model=config["model"]["artifactPath"])
    mx.synchronize()
    load_seconds = time.monotonic() - started
    print(f"mlx model load: {load_seconds:.3f}s", file=sys.stderr, flush=True)
    with output_path.open("w") as output:
        for repeat in range(config.get("repeats", 1)):
            for index, case in enumerate(cases):
                started = time.monotonic()
                try:
                    result = session.transcribe(
                        case["audioPath"],
                        language={"zh": "Chinese", "en": "English"}[case["language"]],
                        return_chunks=True,
                        return_timestamps=False,
                        diarize=False,
                        verbose=False,
                    )
                    mx.synchronize()
                    row = {
                        "schemaVersion": 2,
                        "predictionKind": "audio_host_prediction",
                        "sourceDomain": "audio",
                        "assistantSemanticQualityStatus": "not_assessed",
                        "status": "ok",
                        "text": result.text,
                        "chunks": result.chunks,
                        "wordTimestamps": "not_assessed",
                        "generationTruncated": result.truncated,
                        "finishReason": result.finish_reason,
                    }
                except Exception as error:
                    row = {"status": "failed", "error": str(error)}
                row.update(
                    caseId=case["id"],
                    repeat=repeat,
                    wallSeconds=time.monotonic() - started,
                    modelReloadedPerCall=False,
                    modelLoadSeconds=load_seconds,
                    effectiveDevice=str(mx.default_device()),
                    threads="runtime-auto",
                    firstInference=repeat == 0 and index == 0,
                )
                output.write(json.dumps(row, ensure_ascii=False) + "\n")
                output.flush()
                print(
                    f"mlx: {case['id']} repeat {repeat} {row['wallSeconds']:.3f}s {row['status']}",
                    file=sys.stderr,
                    flush=True,
                )


if __name__ == "__main__":
    main()
