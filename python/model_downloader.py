#!/usr/bin/env python3
"""
Model downloader for ASR backends.

Supports downloading models for:
- Paraformer (FunASR)
- Qwen3-ASR
- MiMo-V2.5-ASR

Outputs JSON progress events to stdout for integration with Rust/Tauri.
"""

import argparse
import json
import sys
from pathlib import Path
from typing import Literal, Optional


def emit_progress(stage: str, progress: float) -> None:
    """Emit a progress event to stdout."""
    event = {"type": "progress", "stage": stage, "progress": progress}
    print(json.dumps(event), flush=True)


def emit_result(success: bool, cache_path: Optional[str] = None, error: Optional[str] = None, code: Optional[str] = None) -> None:
    """Emit a result event to stdout."""
    result = {"type": "result", "success": success}
    if cache_path:
        result["cache_path"] = cache_path
    if error:
        result["error"] = error
    if code:
        result["code"] = code
    print(json.dumps(result), flush=True)


def download_paraformer(model_id: str, hf_token: Optional[str] = None) -> str:
    """
    Download Paraformer model using FunASR.

    Args:
        model_id: Model identifier (e.g., "funasr/paraformer-zh")
        hf_token: Optional HuggingFace token (not typically needed for Paraformer)

    Returns:
        Path to the cached model directory

    Raises:
        ImportError: If funasr is not installed
        RuntimeError: If download fails
    """
    try:
        from funasr import AutoModel
    except ImportError as e:
        raise ImportError("funasr library not found. Install with: pip install funasr") from e

    emit_progress("starting", 0.0)

    try:
        emit_progress("downloading", 0.3)

        # AutoModel will download to ~/.cache/modelscope/ by default
        model = AutoModel(model=model_id, hub="hf")

        emit_progress("loading", 0.8)

        # Determine cache path (modelscope default)
        cache_base = Path.home() / ".cache" / "modelscope" / "hub"
        # FunASR stores models under hub/<model_id>
        model_path = cache_base / model_id.replace("/", "--")

        emit_progress("completed", 1.0)

        return str(model_path)

    except Exception as e:
        raise RuntimeError(f"Failed to download Paraformer model: {e}") from e


def download_qwen3_asr(model_id: str, hf_token: Optional[str] = None) -> str:
    """
    Download Qwen3-ASR model.

    Args:
        model_id: Model identifier (e.g., "Qwen/Qwen3-ASR-1.7B")
        hf_token: Optional HuggingFace token for gated models

    Returns:
        Path to the cached model directory

    Raises:
        ImportError: If qwen_asr is not installed
        RuntimeError: If download fails or GPU is not available
    """
    try:
        from qwen_asr import Qwen3ASRModel
    except ImportError as e:
        raise ImportError("qwen_asr library not found. Install with: pip install qwen-asr") from e

    # Check for GPU availability
    try:
        import torch
        if not torch.cuda.is_available():
            raise RuntimeError("GPU is required for Qwen3-ASR but CUDA is not available")
    except ImportError:
        raise ImportError("PyTorch not found. Install with: pip install torch")

    emit_progress("starting", 0.0)

    try:
        emit_progress("downloading", 0.3)

        # Set HF token if provided
        if hf_token:
            import os
            os.environ["HF_TOKEN"] = hf_token

        # from_pretrained will download to ~/.cache/huggingface/
        model = Qwen3ASRModel.from_pretrained(model_id)

        emit_progress("loading", 0.8)

        # Determine cache path (HuggingFace default)
        cache_base = Path.home() / ".cache" / "huggingface" / "hub"
        # HF stores models as models--<org>--<name>
        model_name = model_id.replace("/", "--")
        model_path = cache_base / f"models--{model_name}"

        emit_progress("completed", 1.0)

        return str(model_path)

    except Exception as e:
        raise RuntimeError(f"Failed to download Qwen3-ASR model: {e}") from e


def download_mimo(model_id: str, hf_token: Optional[str] = None) -> str:
    """
    Download MiMo-V2.5-ASR model using transformers pipeline.

    Args:
        model_id: Model identifier (e.g., "XiaomiMiMo/MiMo-V2.5-ASR")
        hf_token: Optional HuggingFace token for gated models

    Returns:
        Path to the cached model directory

    Raises:
        ImportError: If transformers is not installed
        RuntimeError: If download fails
    """
    try:
        from transformers import pipeline
    except ImportError as e:
        raise ImportError("transformers library not found. Install with: pip install transformers") from e

    emit_progress("starting", 0.0)

    try:
        emit_progress("downloading", 0.3)

        # Create pipeline which will download to ~/.cache/huggingface/
        pipe = pipeline(
            "automatic-speech-recognition",
            model=model_id,
            token=hf_token if hf_token else None
        )

        emit_progress("loading", 0.8)

        # Determine cache path (HuggingFace default)
        cache_base = Path.home() / ".cache" / "huggingface" / "hub"
        # HF stores models as models--<org>--<name>
        model_name = model_id.replace("/", "--")
        model_path = cache_base / f"models--{model_name}"

        emit_progress("completed", 1.0)

        return str(model_path)

    except Exception as e:
        raise RuntimeError(f"Failed to download MiMo model: {e}") from e


def main() -> None:
    """Main entry point for model downloader CLI."""
    parser = argparse.ArgumentParser(
        description="Download ASR models for Paraformer, Qwen3-ASR, or MiMo"
    )
    parser.add_argument(
        "--backend",
        type=str,
        required=True,
        choices=["paraformer", "qwen3-asr", "mimo"],
        help="ASR backend to download model for"
    )
    parser.add_argument(
        "--model-id",
        type=str,
        required=True,
        help="Model identifier (e.g., 'funasr/paraformer-zh', 'Qwen/Qwen3-ASR-1.7B')"
    )
    parser.add_argument(
        "--hf-token",
        type=str,
        default=None,
        help="HuggingFace token for gated models"
    )

    args = parser.parse_args()

    try:
        backend: Literal["paraformer", "qwen3-asr", "mimo"] = args.backend

        if backend == "paraformer":
            cache_path = download_paraformer(args.model_id, args.hf_token)
        elif backend == "qwen3-asr":
            cache_path = download_qwen3_asr(args.model_id, args.hf_token)
        elif backend == "mimo":
            cache_path = download_mimo(args.model_id, args.hf_token)
        else:
            raise ValueError(f"Unknown backend: {backend}")

        emit_result(success=True, cache_path=cache_path)
        sys.exit(0)

    except ImportError as e:
        emit_result(success=False, error=str(e), code="LIBRARY_MISSING")
        sys.exit(1)

    except RuntimeError as e:
        error_msg = str(e)
        if "GPU" in error_msg or "CUDA" in error_msg:
            emit_result(success=False, error=error_msg, code="GPU_REQUIRED")
        else:
            emit_result(success=False, error=error_msg, code="DOWNLOAD_FAILED")
        sys.exit(1)

    except Exception as e:
        emit_result(success=False, error=f"Unexpected error: {e}", code="UNKNOWN_ERROR")
        sys.exit(1)


if __name__ == "__main__":
    main()
