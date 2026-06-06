#!/usr/bin/env python3
"""
meetnote Diarization CLI - pyannote.audio speaker diarization
Usage: uv run python python/diarization_cli.py --audio <path>

Output: JSON to stdout
Progress: lines to stderr prefixed with PROGRESS:
Errors: JSON error object to stdout, non-zero exit
"""
import sys
import json
import argparse
import os
import atexit
import contextlib
import time

from meetnote.device import get_device
from meetnote.progress import emit_progress

# Preserved real-stdout fd, set once _main_impl redirects fd 1 to stderr.
_REAL_STDOUT_FD: int | None = None


def _main_impl():
    hf_token = sys.stdin.readline().strip() or None
    parser = argparse.ArgumentParser(description="meetnote Diarization CLI")
    parser.add_argument("--audio", required=True, help="Path to audio file")
    parser.add_argument("--min-speakers", type=int, default=1, dest="min_speakers", help="Minimum number of speakers")
    parser.add_argument("--max-speakers", type=int, default=10, dest="max_speakers", help="Maximum number of speakers")
    parser.add_argument(
        "--device",
        default="auto",
        choices=["auto", "cpu", "cuda", "mps"],
        help="Inference device: auto, cpu, cuda, mps",
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
        emit_json({"kind": "diarization.error", "schemaVersion": "meetnote.diarization.v1", "code": "DIARIZATION_PROCESS_FAILED", "error": f"Audio file not found: {args.audio}", "message": f"Audio file not found: {args.audio}", "recoverable": False})
        sys.exit(1)

    try:
        from pyannote.audio import Pipeline
    except ImportError:
        emit_json({"kind": "diarization.error", "schemaVersion": "meetnote.diarization.v1", "code": "DIARIZATION_MODEL_MISSING", "error": "pyannote.audio not installed. Run: cd python && uv sync", "message": "pyannote.audio not installed.", "recoverable": True})
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
            token=hf_token,
        )
    except Exception as e:
        err_msg = str(e).lower()
        if "401" in err_msg or "unauthorized" in err_msg:
            code = "DIARIZATION_AUTH_REQUIRED"
        elif "403" in err_msg or "gated" in err_msg or "accept" in err_msg:
            code = "DIARIZATION_AUTH_REQUIRED"
        else:
            code = "DIARIZATION_MODEL_MISSING"
        emit_json({"kind": "diarization.error", "schemaVersion": "meetnote.diarization.v1", "code": code, "error": str(e), "message": str(e), "recoverable": True})
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
        emit_json({"kind": "diarization.error", "schemaVersion": "meetnote.diarization.v1", "code": "DIARIZATION_PROCESS_FAILED", "error": f"Diarization failed: {str(e)}", "message": f"Diarization failed: {str(e)}", "recoverable": False})
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
        "kind": "diarization.result",
        "schemaVersion": "meetnote.diarization.v1",
        "speakers": speakers,
        "segments": segments,
    }

    emit_progress("5/5:done")
    emit_json(output)


def emit_json(payload: dict) -> None:
    """Write one JSON line to the real (un-redirected) stdout."""
    if _REAL_STDOUT_FD is not None:
        with os.fdopen(os.dup(_REAL_STDOUT_FD), "w", closefd=True) as out:
            out.write(json.dumps(payload, ensure_ascii=False))
            out.write("\n")
            out.flush()
    else:
        print(json.dumps(payload, ensure_ascii=False))


def redirect_stdout_to_stderr():
    """Redirect file descriptor 1 to stderr to prevent library stdout pollution."""
    global _REAL_STDOUT_FD
    _REAL_STDOUT_FD = os.dup(1)
    os.dup2(2, 1)
    atexit.register(lambda fd=_REAL_STDOUT_FD: os.close(fd))
    sys.stdout = os.fdopen(2, "w", closefd=False)


def main():
    try:
        redirect_stdout_to_stderr()
        _main_impl()
    except SystemExit:
        raise
    except Exception as e:
        payload = json.dumps({"kind": "diarization.error", "schemaVersion": "meetnote.diarization.v1", "code": "DIARIZATION_PROCESS_FAILED", "message": f"Internal error: {str(e)}", "recoverable": False})
        if _REAL_STDOUT_FD is not None:
            with os.fdopen(os.dup(_REAL_STDOUT_FD), "w", closefd=True) as out:
                out.write(payload + "\n")
                out.flush()
        else:
            print(payload, file=sys.stdout)
        sys.exit(1)


if __name__ == "__main__":
    main()
