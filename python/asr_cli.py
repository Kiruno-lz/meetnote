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
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Optional, Dict, Any

from meetnote.device import get_device
from meetnote.progress import emit_progress

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
    kind: str = "asr.result"
    schemaVersion: str = "meetnote.asr.v1"


def _model_id_to_dir_name(model_id: str) -> str:
    """Map a model source identifier to a cache directory name.

    Examples:
        funasr/paraformer-zh     -> paraformer-zh
        Qwen/Qwen3-ASR-1.7B     -> qwen3-asr-1.7b
        XiaomiMiMo/MiMo-V2.5-ASR -> mimo-v2.5-asr
    """
    mapping = {
        "funasr/paraformer-zh": "paraformer-zh",
        "Qwen/Qwen3-ASR-1.7B": "qwen3-asr-1.7b",
        "XiaomiMiMo/MiMo-V2.5-ASR": "mimo-v2.5-asr",
    }
    if model_id in mapping:
        return mapping[model_id]
    # Fallback: last path component, lowercased
    return model_id.split("/")[-1].lower()


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

        cache_dir = kwargs.get("cache_dir")
        if cache_dir:
            model_path = Path(cache_dir) / f"{model_id}.pt"
            if not model_path.exists():
                raise FileNotFoundError(
                    f"Model not found at {model_path}. Please download it first."
                )
            model_id = str(model_path)

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
                "confidence": round(max(0.0, min(1.0, 1.0 - abs(float(seg.get("avg_logprob", -0.1))))), 3),
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

        cache_dir = kwargs.get("cache_dir")
        if cache_dir:
            dir_name = _model_id_to_dir_name(model_id)
            local_path = Path(cache_dir) / dir_name
            if not local_path.exists():
                raise FileNotFoundError(
                    f"Model not found at {local_path}. Please download it first."
                )
            model_id = str(local_path)

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


class MlxQwen3ASREngine(ASREngine):
    """Qwen3-ASR engine via MLX (Apple Silicon only, ~5GB peak memory).

    Uses MLX instead of PyTorch to load Qwen3-ASR directly on Apple GPU
    at float16, avoiding the float32 + CPU→GPU double allocation that
    causes 30GB+ memory spikes in PyTorch/MPS.
    """

    def __init__(self):
        self.session = None
        self.model_id = None

    def load_model(self, model_id: str, device: str, **kwargs):
        try:
            import mlx.core as mx
            import mlx_qwen3_asr as M
        except ImportError:
            raise ImportError(
                "mlx-qwen3-asr not installed. Run: cd python && uv add mlx-qwen3-asr"
            )

        model, config = M.load_model(model_id, dtype=mx.float16)
        self.session = M.Session(model=model, dtype=mx.float16)
        self.model_id = model_id

    def transcribe(self, audio_path: str, language: str, **kwargs) -> TranscriptionResult:
        if self.session is None:
            raise RuntimeError("Model not loaded")

        lang = None if language == "auto" else language
        result = self.session.transcribe(
            audio_path,
            language=lang,
            return_timestamps=False,
            draft_model=None,
            forced_aligner=None,
            verbose=False,
        )

        segments = []
        # Single text result (no timestamps — avoids forced_aligner download)
        segments.append({
            "id": "asr-seg-0000",
            "startMs": 0,
            "endMs": 0,
            "text": getattr(result, 'text', str(result)).strip(),
            "language": language if language != "auto" else "zh",
            "confidence": 1.0,
        })

        return TranscriptionResult(
            segments=segments,
            language=getattr(result, 'language', language or "zh"),
            model=kwargs.get("model_id", "qwen3-asr-mlx"),
            backend="qwen3-asr"
        )


class Qwen3ASREngine(ASREngine):
    """Qwen3-ASR engine. Used as fallback when MLX is not available."""

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

        # Qwen3-ASR uses device_map.
        # CUDA: device_map="auto" with bfloat16.
        # MPS (Apple Silicon): device_map=None (loads on CPU/MPS with float32)
        # CPU: device_map="cpu" with float32.
        if device == "cuda":
            device_map = "auto"
            dtype = torch.bfloat16
        elif device == "mps":
            device_map = None      # MPS doesn't support device_map; use manual to()
            dtype = torch.float32
        else:
            device_map = "cpu"
            dtype = torch.float32

        cache_dir = kwargs.get("cache_dir")
        if cache_dir:
            dir_name = _model_id_to_dir_name(model_id)
            local_path = Path(cache_dir) / dir_name
            if not local_path.exists():
                raise FileNotFoundError(
                    f"Model not found at {local_path}. Please download it first."
                )
            model_id = str(local_path)

        self.model = Qwen3ASRModel.from_pretrained(
            model_id,
            dtype=dtype,
            device_map=device_map,
            trust_remote_code=True,
        )

        if device == "mps" and self.model is not None:
            try:
                self.model = self.model.to("mps")
            except Exception as e:
                print(f"WARN: moving Qwen3-ASR to MPS failed ({e}), falling back to CPU", file=sys.stderr, flush=True)
                self.model = self.model.to("cpu")

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
    """Xiaomi MiMo-V2.5-ASR engine.

    MiMo-V2.5-ASR is a Qwen2-based ASR model that requires a custom audio
    tokenizer and the ``MimoAudio`` pipeline from Xiaomi's GitHub repository
    (https://github.com/XiaomiMiMo/MiMo-V2.5-ASR). The standard
    ``transformers.pipeline("automatic-speech-recognition")`` does not support
    this architecture.

    The correct integration requires:

    1. Cloning the MiMo-V2.5-ASR repo and installing its dependencies.
    2. Downloading the separate ``MiMo-Audio-Tokenizer`` model.
    3. Using ``from src.mimo_audio.mimo_audio import MimoAudio``.

    Until that integration is complete, this engine provides a clear error
    message pointing to the required setup steps.
    """

    def __init__(self):
        self.model = None

    def load_model(self, model_id: str, device: str, **kwargs):
        cache_dir = kwargs.get("cache_dir")
        if cache_dir:
            dir_name = _model_id_to_dir_name(model_id)
            local_path = Path(cache_dir) / dir_name
            if not local_path.exists():
                raise FileNotFoundError(
                    f"Model not found at {local_path}. Please download it first."
                )
        raise RuntimeError(
            "MiMo-V2.5-ASR is not yet supported via the standard transformers pipeline. "
            "Its architecture (MiMoV2ASRForCausalLM) combines a speech encoder with a "
            "Qwen2 language model and requires a custom audio tokenizer.\n\n"
            "To use MiMo, please:\n"
            "1. Clone https://github.com/XiaomiMiMo/MiMo-V2.5-ASR\n"
            "2. Install its dependencies: pip install -r requirements.txt\n"
            "3. Download the audio tokenizer:\n"
            "   huggingface-cli download XiaomiMiMo/MiMo-Audio-Tokenizer --local-dir models/MiMo-Audio-Tokenizer\n"
            "4. Use the MimoAudio class:\n"
            "   from src.mimo_audio.mimo_audio import MimoAudio\n"
            "   model = MimoAudio(model_path='XiaomiMiMo/MiMo-V2.5-ASR', tokenizer_path='models/MiMo-Audio-Tokenizer')\n"
            "   text = model.asr_sft('audio.wav')\n\n"
            "Support for MiMo will be added in a future release."
        )

    def transcribe(self, audio_path: str, language: str, **kwargs) -> TranscriptionResult:
        raise RuntimeError(
            "MiMo is not yet supported — load_model will provide setup instructions."
        )


def create_engine(backend: str, mlx_override: Optional[bool] = None) -> ASREngine:
    """Factory function to create ASR engine by backend name.

    For qwen3-asr, auto-detects MLX (Apple Silicon) which uses ~5GB peak
    memory vs ~30GB+ for PyTorch/MPS. Falls back to PyTorch when MLX
    is unavailable or when device=cpu.

    Args:
        backend: Engine name ("whisper", "paraformer", "qwen3-asr", "mimo").
        mlx_override: Force MLX (True), force PyTorch (False), or auto (None).
    """
    if backend == "qwen3-asr":
        use_mlx = mlx_override
        if use_mlx is None:
            # Auto-detect: prefer MLX when available on Apple Silicon
            try:
                import mlx_qwen3_asr  # type: ignore
                use_mlx = True
            except ImportError:
                use_mlx = False
        if use_mlx:
            return MlxQwen3ASREngine()

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
    parser.add_argument(
        "--request-id",
        default=None,
        help="Request ID for traceability (passed through to output)"
    )
    parser.add_argument(
        "--cache-dir",
        default=None,
        help="Unified model cache directory (default: use library defaults). "
             "When provided, models are loaded from {cache_dir}/{model_id}.pt "
             "(whisper) or {cache_dir}/{dir_name}/ (other backends)."
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

    # Resolve cache directory
    cache_dir: Optional[str] = None
    if args.cache_dir:
        cache_dir = os.path.expanduser(args.cache_dir)

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
    emit_progress("0/4:loading-model")

    try:
        engine = create_engine(args.backend, mlx_override=False if args.device == "cpu" else None)
    except ValueError as e:
        emit_json({"kind": "asr.error", "schemaVersion": "meetnote.asr.v1", "code": "ASR_PROCESS_FAILED", "error": str(e), "message": str(e), "recoverable": False})
        sys.exit(1)

    # Load model
    try:
        engine.load_model(
            model_id,
            device,
            cpu_threads=args.cpu_threads,
            cache_dir=cache_dir,
        )
    except ImportError as e:
        emit_json({"kind": "asr.error", "schemaVersion": "meetnote.asr.v1", "code": "ASR_MODEL_MISSING", "error": str(e), "message": str(e), "recoverable": True})
        sys.exit(1)
    except FileNotFoundError as e:
        emit_json({"kind": "asr.error", "schemaVersion": "meetnote.asr.v1", "code": "ASR_MODEL_MISSING", "error": str(e), "message": str(e), "recoverable": True})
        sys.exit(1)
    except Exception as e:
        emit_json({"kind": "asr.error", "schemaVersion": "meetnote.asr.v1", "code": "ASR_PROCESS_FAILED", "error": f"Failed to load model '{model_id}': {str(e)}", "message": f"Failed to load model '{model_id}': {str(e)}", "recoverable": False})
        sys.exit(1)

    # Transcribe
    emit_progress("1/4:transcribing")

    try:
        result = engine.transcribe(
            args.audio,
            args.language,
            task=args.task,
            model_id=model_id
        )
    except Exception as e:
        emit_json({"kind": "asr.error", "schemaVersion": "meetnote.asr.v1", "code": "ASR_PROCESS_FAILED", "error": f"Transcription failed: {str(e)}", "message": f"Transcription failed: {str(e)}", "recoverable": False})
        sys.exit(1)

    emit_progress("3/4:finalizing")

    # Output result with contract fields
    output = {
        "kind": "asr.result",
        "schemaVersion": "meetnote.asr.v1",
        "segments": result.segments,
        "language": result.language,
        "model": result.model,
        "backend": result.backend,
    }
    if args.request_id:
        output["requestId"] = args.request_id

    emit_progress("4/4:done")
    emit_json(output)


def main():
    try:
        _main_impl()
    except SystemExit:
        raise
    except Exception as e:
        # Emit to the real stdout: if _main_impl already redirected fd 1 to stderr,
        # use the preserved fd; otherwise sys.stdout is still the real stdout.
        payload = json.dumps({"kind": "asr.error", "schemaVersion": "meetnote.asr.v1", "code": "ASR_PROCESS_FAILED", "message": f"Internal error: {str(e)}", "recoverable": False})
        if _REAL_STDOUT_FD is not None:
            with os.fdopen(os.dup(_REAL_STDOUT_FD), "w", closefd=True) as out:
                out.write(payload + "\n")
                out.flush()
        else:
            print(payload, file=sys.stdout)
        sys.exit(1)


if __name__ == "__main__":
    main()
