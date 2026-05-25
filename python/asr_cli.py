#!/usr/bin/env python3
"""
meetnote ASR CLI - Whisper-based transcription
Usage: uv run python python/asr_cli.py --audio <path> [--model <model>] [--language <lang>]

Output: JSON to stdout
Progress: lines to stderr prefixed with PROGRESS:
Errors: JSON error object to stdout, non-zero exit
"""
import sys
import json
import argparse
import os
import contextlib


def get_device(requested: str) -> str:
    """Resolve requested device to an actual available device."""
    import torch
    if requested == "auto":
        if torch.backends.mps.is_available():
            return "mps"
        return "cpu"
    elif requested == "mps":
        if not torch.backends.mps.is_available():
            print(json.dumps({"error": "MPS not available on this system"}))
            sys.exit(1)
        return "mps"
    return "cpu"


def _main_impl():
    parser = argparse.ArgumentParser(description="meetnote ASR CLI")
    parser.add_argument("--audio", required=True, help="Path to audio file")
    parser.add_argument("--model", default="large-v3", help="Whisper model name or path to ggml file")
    parser.add_argument("--language", default="zh", help="Language code (zh, en, auto)")
    parser.add_argument("--task", default="transcribe", choices=["transcribe", "translate"])
    parser.add_argument(
        "--device",
        default="auto",
        choices=["auto", "cpu", "mps", "cpu_multithreaded"],
        help="Inference device: auto (MPS if available, else CPU), cpu, mps, cpu_multithreaded",
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

    # Try to import whisper
    try:
        import whisper
    except ImportError:
        print(json.dumps({
            "error": "openai-whisper not installed. Run: cd python && uv sync",
            "hint": "pip install openai-whisper"
        }))
        sys.exit(1)

    print("PROGRESS:0/4:loading-model", file=sys.stderr, flush=True)

    device = get_device(args.device)
    if args.device == "cpu_multithreaded":
        import torch
        cpu_count = os.cpu_count() or 4
        torch.set_num_threads(cpu_count)
    elif device == "cpu" and args.cpu_threads > 0:
        import torch
        torch.set_num_threads(args.cpu_threads)

    try:
        model = whisper.load_model(args.model, device=device)
    except Exception as e:
        if device == "mps":
            # MPS fallback: some PyTorch versions have fp16/kernel issues on MPS
            print(f"WARN: MPS load failed ({e}), retrying on CPU", file=sys.stderr, flush=True)
            device = "cpu"
            try:
                model = whisper.load_model(args.model, device=device)
            except Exception as e2:
                print(json.dumps({"error": f"Failed to load model '{args.model}': {str(e2)}"}))
                sys.exit(1)
        else:
            print(json.dumps({"error": f"Failed to load model '{args.model}': {str(e)}"}))
            sys.exit(1)

    print("PROGRESS:1/4:transcribing", file=sys.stderr, flush=True)

    language = None if args.language == "auto" else args.language

    try:
        with contextlib.redirect_stdout(sys.stderr):
            result = model.transcribe(
                args.audio,
                language=language,
                task=args.task,
                verbose=False,
                word_timestamps=False,
                fp16=False,
            )
    except Exception as e:
        print(json.dumps({"error": f"Transcription failed: {str(e)}"}))
        sys.exit(1)

    print("PROGRESS:3/4:finalizing", file=sys.stderr, flush=True)

    segments = []
    for i, seg in enumerate(result.get("segments", [])):
        detected_lang = result.get("language", args.language or "zh")
        if detected_lang in ("zh", "en"):
            seg_lang = detected_lang
        else:
            seg_lang = "unknown"
        segments.append({
            "id": f"asr-seg-{i:04d}",
            "startMs": int(float(seg.get("start", 0)) * 1000),
            "endMs": int(float(seg.get("end", 0)) * 1000),
            "text": seg.get("text", "").strip(),
            "language": seg_lang,
            "confidence": round(min(1.0, max(0.0, abs(float(seg.get("avg_logprob", -0.1))))), 3),
        })

    output = {
        "segments": segments,
        "language": result.get("language", args.language or "zh"),
        "model": args.model,
    }

    print("PROGRESS:4/4:done", file=sys.stderr, flush=True)
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
