#!/usr/bin/env python3
"""
meetnote Diarization CLI - pyannote.audio speaker diarization
Usage: uv run python python/diarization_cli.py --audio <path> --hf-token <token>

Output: JSON to stdout
Progress: lines to stderr prefixed with PROGRESS:
Errors: JSON error object to stdout, non-zero exit
"""
import sys
import json
import argparse
import os
import contextlib
import time

from meetnote.device import get_device
from meetnote.progress import emit_progress


def _main_impl():
    parser = argparse.ArgumentParser(description="meetnote Diarization CLI")
    parser.add_argument("--audio", required=True, help="Path to audio file")
    parser.add_argument("--hf-token", required=True, dest="hf_token", help="HuggingFace access token")
    parser.add_argument("--min-speakers", type=int, default=1, dest="min_speakers", help="Minimum number of speakers")
    parser.add_argument("--max-speakers", type=int, default=10, dest="max_speakers", help="Maximum number of speakers")
    parser.add_argument(
        "--device",
        default="auto",
        choices=["auto", "cpu", "mps"],
        help="Inference device: auto (MPS if available, else CPU), cpu, mps",
    )
    parser.add_argument(
        "--cpu-threads",
        type=int,
        default=0,
        dest="cpu_threads",
        help="Number of CPU threads for torch (0 = let PyTorch decide)",
    )
    args = parser.parse_args()

    if not os.path.exists(args.audio):
        print(json.dumps({"error": f"Audio file not found: {args.audio}"}))
        sys.exit(1)

    try:
        from pyannote.audio import Pipeline
    except ImportError:
        print(json.dumps({
            "error": "pyannote.audio not installed. Run: cd python && uv sync",
        }))
        sys.exit(1)

    emit_progress("0/5:queued")
    time.sleep(0.3)
    emit_progress("1/5:loading-model")

    device = get_device(args.device)
    if device == "cpu" and args.cpu_threads > 0:
        import torch
        torch.set_num_threads(args.cpu_threads)

    try:
        pipeline = Pipeline.from_pretrained(
            "pyannote/speaker-diarization-3.1",
            token=args.hf_token,
        )
    except Exception as e:
        err_msg = str(e).lower()
        if "401" in err_msg or "unauthorized" in err_msg:
            print(json.dumps({
                "error": f"HuggingFace authentication failed: {str(e)}",
                "code": "DIARIZATION_AUTH_REQUIRED",
            }))
        elif "403" in err_msg or "gated" in err_msg or "accept" in err_msg:
            print(json.dumps({
                "error": (
                    f"HuggingFace model access denied (403). You must accept the model terms at: "
                    f"https://huggingface.co/pyannote/speaker-diarization-3.1 "
                    f"and https://huggingface.co/pyannote/segmentation-3.0 — {str(e)}"
                ),
                "code": "DIARIZATION_MODEL_GATED",
            }))
        else:
            print(json.dumps({
                "error": f"Failed to load diarization model: {str(e)}",
                "code": "DIARIZATION_MODEL_MISSING",
            }))
        sys.exit(1)

    import torch
    pipeline.to(torch.device(device))

    emit_progress("2/5:segmenting")
    time.sleep(0.3)

    try:
        with contextlib.redirect_stdout(sys.stderr):
            result = pipeline(
                args.audio,
                min_speakers=args.min_speakers,
                max_speakers=args.max_speakers,
            )
    except Exception as e:
        print(json.dumps({
            "error": f"Diarization failed: {str(e)}",
            "code": "DIARIZATION_PROCESS_FAILED",
        }))
        sys.exit(1)

    emit_progress("3/5:clustering")
    time.sleep(0.5)
    emit_progress("4/5:finalizing")
    time.sleep(0.3)

    # pyannote.audio 4.x returns DiarizeOutput; 3.x returns Annotation directly
    diarization = result.speaker_diarization if hasattr(result, "speaker_diarization") else result

    raw_segments = []
    speaker_set: set[str] = set()
    for segment, _, speaker in diarization.itertracks(yield_label=True):
        speaker_set.add(speaker)
        raw_segments.append({
            "speaker": speaker,
            "startMs": int(round(segment.start * 1000)),
            "endMs": int(round(segment.end * 1000)),
        })

    speaker_map = {}
    speakers = []
    for idx, spk in enumerate(sorted(speaker_set), start=1):
        sid = f"speaker-{idx}"
        speaker_map[spk] = sid
        speakers.append({"id": sid, "label": f"Speaker {idx}"})

    raw_segments.sort(key=lambda s: (s["startMs"], s["endMs"]))

    segments = []
    for i, seg in enumerate(raw_segments):
        overlap = False
        if i > 0:
            prev = raw_segments[i - 1]
            if seg["startMs"] < prev["endMs"]:
                overlap = True

        segments.append({
            "id": f"dia-seg-{i + 1:04d}",
            "speakerId": speaker_map[seg["speaker"]],
            "startMs": seg["startMs"],
            "endMs": seg["endMs"],
            "confidence": 0.9,
            "overlap": overlap,
        })

    output = {
        "speakers": speakers,
        "segments": segments,
    }

    emit_progress("5/5:done")
    print(json.dumps(output, ensure_ascii=False))


def main():
    try:
        _main_impl()
    except SystemExit:
        raise
    except Exception as e:
        print(json.dumps({"error": f"Internal error: {str(e)}"}))
        sys.exit(1)

if __name__ == "__main__":
    main()
