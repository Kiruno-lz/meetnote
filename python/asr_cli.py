#!/usr/bin/env python3
"""
meetnote ASR CLI - Unified multi-backend transcription
Supports: Whisper, Paraformer, Qwen3-ASR, MiMo-V2.5-ASR

Usage: uv run python python/asr_cli.py --audio <path> --backend <backend> [options]

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
from abc import ABC, abstractmethod
from dataclasses import dataclass, asdict
from typing import List, Optional, Dict, Any

# Preserved real-stdout fd, set once _main_impl redirects fd 1 to stderr so the
# JSON result is never corrupted by third-party library banners. main()'s fallback
# error handler uses it to still reach the real stdout. None until redirect happens.
_REAL_STDOUT_FD: Optional[int] = None


@dataclass
class Segment:
    """Unified segment representation across all ASR backends."""
    id: str
    startMs: int
    endMs: int
    text: str
    language: str
    confidence: float


@dataclass
class TranscriptionResult:
    """Unified transcription result."""
    segments: List[Dict[str, Any]]
    language: str
    model: str
    backend: str


class ASREngine(ABC):
    """Abstract base class for ASR engines."""

    @abstractmethod
    def load_model(self, model_id: str, device: str, **kwargs):
        """Load the ASR model."""
        pass

    @abstractmethod
    def transcribe(self, audio_path: str, language: str, **kwargs) -> TranscriptionResult:
        """Transcribe audio file and return unified result."""
        pass


class WhisperEngine(ASREngine):
    """OpenAI Whisper ASR engine."""

    def __init__(self):
        self.model = None
        self.device = None

    def load_model(self, model_id: str, device: str, **kwargs):
        """Load Whisper model."""
        try:
            import whisper
        except ImportError:
            raise ImportError(
                "openai-whisper not installed. Run: cd python && uv sync\n"
                "Or: pip install openai-whisper"
            )

        cpu_threads = kwargs.get("cpu_threads", 0)
        if device == "cpu" and cpu_threads > 0:
            import torch
            torch.set_num_threads(cpu_threads)

        try:
            self.model = whisper.load_model(model_id, device=device)
            self.device = device
        except Exception as e:
            if device == "mps":
                # MPS fallback: some PyTorch versions have fp16/kernel issues on MPS
                print(f"WARN: MPS load failed ({e}), retrying on CPU", file=sys.stderr, flush=True)
                device = "cpu"
                self.model = whisper.load_model(model_id, device=device)
                self.device = device
            else:
                raise

    def transcribe(self, audio_path: str, language: str, **kwargs) -> TranscriptionResult:
        """Transcribe using Whisper."""
        if self.model is None:
            raise RuntimeError("Model not loaded")

        task = kwargs.get("task", "transcribe")
        lang = None if language == "auto" else language

        with contextlib.redirect_stdout(sys.stderr):
            result = self.model.transcribe(
                audio_path,
                language=lang,
                task=task,
                verbose=False,
                word_timestamps=False,
                fp16=False,
            )

        segments = []
        for i, seg in enumerate(result.get("segments", [])):
            detected_lang = result.get("language", language or "zh")
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

        return TranscriptionResult(
            segments=segments,
            language=result.get("language", language or "zh"),
            model=kwargs.get("model_id", "whisper"),
            backend="whisper"
        )


class ParaformerEngine(ASREngine):
    """FunASR Paraformer engine."""

    def __init__(self):
        self.model = None

    def load_model(self, model_id: str, device: str, **kwargs):
        """Load Paraformer model via FunASR."""
        try:
            from funasr import AutoModel
        except ImportError:
            raise ImportError(
                "funasr not installed. Run: cd python && uv add funasr\n"
                "Or: pip install funasr"
            )

        # FunASR supports "cuda" or "cpu". It has no MPS backend, so Apple Silicon
        # (where auto-detect yields "mps") must fall back to CPU rather than request
        # a CUDA device that doesn't exist.
        funasr_device = "cuda" if device == "cuda" else "cpu"

        self.model = AutoModel(
            model=model_id,
            hub="hf",
            device=funasr_device
        )

    def transcribe(self, audio_path: str, language: str, **kwargs) -> TranscriptionResult:
        """Transcribe using Paraformer."""
        if self.model is None:
            raise RuntimeError("Model not loaded")

        result = self.model.generate(input=audio_path)

        segments = []
        # FunASR returns list of dicts with 'text' and optional timestamps
        if isinstance(result, list):
            for i, item in enumerate(result):
                if isinstance(item, dict):
                    text = item.get("text", "")
                    start_ms = int(item.get("timestamp", [0, 0])[0] * 1000) if "timestamp" in item else 0
                    end_ms = int(item.get("timestamp", [0, 0])[1] * 1000) if "timestamp" in item else 0
                else:
                    text = str(item)
                    start_ms = 0
                    end_ms = 0

                segments.append({
                    "id": f"asr-seg-{i:04d}",
                    "startMs": start_ms,
                    "endMs": end_ms,
                    "text": text.strip(),
                    "language": language if language != "auto" else "zh",
                    "confidence": 1.0,  # FunASR doesn't provide confidence by default
                })
        else:
            # Single string result
            segments.append({
                "id": "asr-seg-0000",
                "startMs": 0,
                "endMs": 0,
                "text": str(result).strip(),
                "language": language if language != "auto" else "zh",
                "confidence": 1.0,
            })

        return TranscriptionResult(
            segments=segments,
            language=language if language != "auto" else "zh",
            model=kwargs.get("model_id", "paraformer"),
            backend="paraformer"
        )


class Qwen3ASREngine(ASREngine):
    """Qwen3-ASR engine."""

    def __init__(self):
        self.model = None

    def load_model(self, model_id: str, device: str, **kwargs):
        """Load Qwen3-ASR model."""
        try:
            from qwen_asr import Qwen3ASRModel
            import torch
        except ImportError:
            raise ImportError(
                "qwen-asr not installed. Run: cd python && uv add qwen-asr\n"
                "Or: pip install qwen-asr"
            )

        # Qwen3-ASR uses device_map
        device_map = "auto" if device == "cuda" else "cpu"
        dtype = torch.bfloat16 if device == "cuda" else torch.float32

        self.model = Qwen3ASRModel.from_pretrained(
            model_id,
            dtype=dtype,
            device_map=device_map
        )

    def transcribe(self, audio_path: str, language: str, **kwargs) -> TranscriptionResult:
        """Transcribe using Qwen3-ASR."""
        if self.model is None:
            raise RuntimeError("Model not loaded")

        results = self.model.transcribe(audio=audio_path)

        segments = []
        # Qwen3-ASR returns dict with 'text' and optional 'segments'
        if isinstance(results, dict):
            if "segments" in results:
                for i, seg in enumerate(results["segments"]):
                    segments.append({
                        "id": f"asr-seg-{i:04d}",
                        "startMs": int(seg.get("start", 0) * 1000),
                        "endMs": int(seg.get("end", 0) * 1000),
                        "text": seg.get("text", "").strip(),
                        "language": language if language != "auto" else "zh",
                        "confidence": seg.get("confidence", 1.0),
                    })
            else:
                # Single text result
                segments.append({
                    "id": "asr-seg-0000",
                    "startMs": 0,
                    "endMs": 0,
                    "text": results.get("text", "").strip(),
                    "language": language if language != "auto" else "zh",
                    "confidence": 1.0,
                })

        return TranscriptionResult(
            segments=segments,
            language=language if language != "auto" else "zh",
            model=kwargs.get("model_id", "qwen3-asr"),
            backend="qwen3-asr"
        )


class MiMoEngine(ASREngine):
    """Xiaomi MiMo-V2.5-ASR engine via transformers pipeline."""

    def __init__(self):
        self.pipe = None

    def load_model(self, model_id: str, device: str, **kwargs):
        """Load MiMo model via transformers pipeline."""
        try:
            from transformers import pipeline
        except ImportError:
            raise ImportError(
                "transformers not installed. Run: cd python && uv add transformers\n"
                "Or: pip install transformers"
            )

        # transformers pipeline uses device index or "cpu"
        pipe_device = 0 if device == "cuda" else -1

        self.pipe = pipeline(
            "automatic-speech-recognition",
            model=model_id,
            device=pipe_device
        )

    def transcribe(self, audio_path: str, language: str, **kwargs) -> TranscriptionResult:
        """Transcribe using MiMo."""
        if self.pipe is None:
            raise RuntimeError("Model not loaded")

        result = self.pipe(audio_path)

        segments = []
        # transformers ASR pipeline returns dict with 'text' and optional 'chunks'
        if isinstance(result, dict):
            if "chunks" in result:
                for i, chunk in enumerate(result["chunks"]):
                    segments.append({
                        "id": f"asr-seg-{i:04d}",
                        "startMs": int(chunk.get("timestamp", [0, 0])[0] * 1000),
                        "endMs": int(chunk.get("timestamp", [0, 0])[1] * 1000),
                        "text": chunk.get("text", "").strip(),
                        "language": language if language != "auto" else "zh",
                        "confidence": 1.0,
                    })
            else:
                # Single text result
                segments.append({
                    "id": "asr-seg-0000",
                    "startMs": 0,
                    "endMs": 0,
                    "text": result.get("text", "").strip(),
                    "language": language if language != "auto" else "zh",
                    "confidence": 1.0,
                })

        return TranscriptionResult(
            segments=segments,
            language=language if language != "auto" else "zh",
            model=kwargs.get("model_id", "mimo"),
            backend="mimo"
        )


def get_device(requested: str) -> str:
    """Resolve requested device to an actual available device."""
    import torch

    if requested == "auto":
        if torch.cuda.is_available():
            return "cuda"
        elif torch.backends.mps.is_available():
            return "mps"
        return "cpu"
    elif requested == "cuda":
        if not torch.cuda.is_available():
            print(json.dumps({"error": "CUDA not available on this system"}), file=sys.stdout)
            sys.exit(1)
        return "cuda"
    elif requested == "mps":
        if not torch.backends.mps.is_available():
            print(json.dumps({"error": "MPS not available on this system"}), file=sys.stdout)
            sys.exit(1)
        return "mps"
    return "cpu"


def create_engine(backend: str) -> ASREngine:
    """Factory function to create ASR engine by backend name."""
    engines = {
        "whisper": WhisperEngine,
        "paraformer": ParaformerEngine,
        "qwen3-asr": Qwen3ASREngine,
        "mimo": MiMoEngine,
    }

    if backend not in engines:
        raise ValueError(f"Unknown backend: {backend}. Available: {', '.join(engines.keys())}")

    return engines[backend]()


def _main_impl():
    parser = argparse.ArgumentParser(
        description="meetnote ASR CLI - Unified multi-backend transcription"
    )
    parser.add_argument("--audio", required=True, help="Path to audio file")
    parser.add_argument(
        "--backend",
        default="whisper",
        choices=["whisper", "paraformer", "qwen3-asr", "mimo"],
        help="ASR backend to use (default: whisper)"
    )
    parser.add_argument(
        "--model",
        default=None,
        help="Model ID or path (default varies by backend)"
    )
    parser.add_argument(
        "--language",
        default="zh",
        help="Language code: zh, en, auto (default: zh)"
    )
    parser.add_argument(
        "--task",
        default="transcribe",
        choices=["transcribe", "translate"],
        help="Task type (whisper only, default: transcribe)"
    )
    parser.add_argument(
        "--device",
        default="auto",
        choices=["auto", "cpu", "cuda", "mps"],
        help="Inference device: auto (CUDA > MPS > CPU), cpu, cuda, mps (default: auto)"
    )
    parser.add_argument(
        "--cpu-threads",
        type=int,
        default=0,
        dest="cpu_threads",
        help="Number of CPU threads for torch (0 = let PyTorch decide)"
    )

    args = parser.parse_args()

    # Validate audio file
    if not os.path.exists(args.audio):
        print(json.dumps({"error": f"Audio file not found: {args.audio}"}), file=sys.stdout)
        sys.exit(1)

    # Set default models per backend
    default_models = {
        "whisper": "large-v3",
        "paraformer": "funasr/paraformer-zh",
        "qwen3-asr": "Qwen/Qwen3-ASR-1.7B",
        "mimo": "XiaomiMiMo/MiMo-V2.5-ASR",
    }
    model_id = args.model or default_models[args.backend]

    # Resolve device
    device = get_device(args.device)

    # Redirect stdout to stderr at the file-descriptor level during model load and
    # inference. Third-party ASR libraries (funasr, qwen-asr, transformers) and their
    # native extensions print version banners / progress bars straight to stdout, which
    # would corrupt the JSON result the Rust bridge parses. We duplicate the real stdout
    # fd, point fd 1 at stderr, and restore it only to emit the final JSON line.
    real_stdout_fd = os.dup(1)
    os.dup2(2, 1)
    # Close the preserved fd on process exit so it never leaks across the CLI's lifetime.
    atexit.register(lambda fd=real_stdout_fd: os.close(fd))
    # Reassign sys.stdout to a stream backed by stderr so Python-level prints follow too.
    sys.stdout = os.fdopen(2, "w", closefd=False)
    # Expose the preserved fd so main()'s last-resort error handler can still emit JSON
    # to the real stdout even after the redirect is in place.
    global _REAL_STDOUT_FD
    _REAL_STDOUT_FD = real_stdout_fd

    def emit_json(payload: dict) -> None:
        """Write one JSON line to the real (un-redirected) stdout."""
        with os.fdopen(os.dup(real_stdout_fd), "w", closefd=True) as out:
            out.write(json.dumps(payload, ensure_ascii=False))
            out.write("\n")
            out.flush()

    # CPU threading setup
    if device == "cpu" and args.cpu_threads > 0:
        import torch
        torch.set_num_threads(args.cpu_threads)

    # Create engine
    print("PROGRESS:0/4:loading-model", file=sys.stderr, flush=True)

    try:
        engine = create_engine(args.backend)
    except ValueError as e:
        emit_json({"error": str(e)})
        sys.exit(1)

    # Load model
    try:
        engine.load_model(
            model_id,
            device,
            cpu_threads=args.cpu_threads
        )
    except ImportError as e:
        emit_json({"error": str(e)})
        sys.exit(1)
    except Exception as e:
        emit_json({"error": f"Failed to load model '{model_id}': {str(e)}"})
        sys.exit(1)

    # Transcribe
    print("PROGRESS:1/4:transcribing", file=sys.stderr, flush=True)

    try:
        result = engine.transcribe(
            args.audio,
            args.language,
            task=args.task,
            model_id=model_id
        )
    except Exception as e:
        emit_json({"error": f"Transcription failed: {str(e)}"})
        sys.exit(1)

    print("PROGRESS:3/4:finalizing", file=sys.stderr, flush=True)

    # Output result
    output = {
        "segments": result.segments,
        "language": result.language,
        "model": result.model,
        "backend": result.backend,
    }

    print("PROGRESS:4/4:done", file=sys.stderr, flush=True)
    emit_json(output)


def main():
    try:
        _main_impl()
    except SystemExit:
        raise
    except Exception as e:
        # Emit to the real stdout: if _main_impl already redirected fd 1 to stderr,
        # use the preserved fd; otherwise sys.stdout is still the real stdout.
        payload = json.dumps({"error": f"Internal error: {str(e)}"})
        if _REAL_STDOUT_FD is not None:
            with os.fdopen(os.dup(_REAL_STDOUT_FD), "w", closefd=True) as out:
                out.write(payload + "\n")
                out.flush()
        else:
            print(payload, file=sys.stdout)
        sys.exit(1)


if __name__ == "__main__":
    main()
