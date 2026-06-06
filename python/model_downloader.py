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
from typing import Optional


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


def download_paraformer(model_id: str, hf_token: Optional[str] = None, cache_dir: Optional[str] = None) -> str:
    """
    Download Paraformer model using huggingface_hub snapshot_download.

    Uses snapshot_download to fetch files without loading the model into
    memory, so no GPU is required during download. Downloads to the unified
    cache_dir when provided, otherwise to the default HF cache.

    Args:
        model_id: Model identifier (e.g., "funasr/paraformer-zh")
        hf_token: Optional HuggingFace token (not typically needed for Paraformer)
        cache_dir: Optional unified cache directory. When set, downloads to
                   <cache_dir>/paraformer-zh/ instead of the HF cache.

    Returns:
        Path to the cached model directory

    Raises:
        ImportError: If huggingface_hub is not installed
        RuntimeError: If download fails
    """
    try:
        from huggingface_hub import snapshot_download
    except ImportError:
        raise ImportError("huggingface_hub not found. Install with: pip install huggingface-hub") from None

    emit_progress("starting", 0.0)

    try:
        if hf_token:
            import os
            os.environ["HF_TOKEN"] = hf_token

        local_dir = Path(cache_dir) / "paraformer-zh" if cache_dir else None
        if local_dir:
            local_dir.mkdir(parents=True, exist_ok=True)

        emit_progress("downloading", 0.1)

        snapshot_download(
            repo_id=model_id,
            local_dir=str(local_dir) if local_dir else None,
            token=hf_token,
            resume_download=True,
        )

        emit_progress("completed", 1.0)

        if local_dir:
            return str(local_dir)
        else:
            # Determine HF cache path for backwards compat
            cache_base = Path.home() / ".cache" / "huggingface" / "hub"
            model_name = model_id.replace("/", "--")
            return str(cache_base / f"models--{model_name}")

    except Exception as e:
        raise RuntimeError(f"Failed to download Paraformer model: {e}") from e


def download_qwen3_asr(model_id: str, hf_token: Optional[str] = None, cache_dir: Optional[str] = None) -> str:
    """
    Download Qwen3-ASR model weights.
    Uses snapshot_download to fetch files without loading the model into
    memory, so no GPU is required during download.

    Args:
        model_id: HuggingFace repo id (e.g., "Qwen/Qwen3-ASR-1.7B")
        hf_token: Optional HuggingFace token for gated models
        cache_dir: Optional unified cache directory. When set, downloads to
                   <cache_dir>/qwen3-asr-1.7b/ instead of the HF cache.

    Returns:
        Path to the cached model directory

    Raises:
        ImportError: If huggingface_hub is not installed
        RuntimeError: If download fails
    """
    try:
        from huggingface_hub import snapshot_download
    except ImportError:
        raise ImportError("huggingface_hub not found. Install with: pip install huggingface-hub") from None

    emit_progress("starting", 0.0)

    try:
        if hf_token:
            import os
            os.environ["HF_TOKEN"] = hf_token

        local_dir = Path(cache_dir) / "qwen3-asr-1.7b" if cache_dir else None
        if local_dir:
            local_dir.mkdir(parents=True, exist_ok=True)

        emit_progress("downloading", 0.3)

        snapshot_download(
            repo_id=model_id,
            local_dir=str(local_dir) if local_dir else None,
            token=hf_token,
            resume_download=True,
        )

        emit_progress("completed", 1.0)

        if local_dir:
            return str(local_dir)
        else:
            # Determine HF cache path for backwards compat
            cache_base = Path.home() / ".cache" / "huggingface" / "hub"
            model_name = model_id.replace("/", "--")
            return str(cache_base / f"models--{model_name}")

    except Exception as e:
        raise RuntimeError(f"Failed to download Qwen3-ASR model: {e}") from e


def download_mimo(model_id: str, hf_token: Optional[str] = None, cache_dir: Optional[str] = None) -> str:
    """
    Download MiMo-V2.5-ASR model weights.
    Uses snapshot_download to fetch files without loading the model into
    memory, so no GPU is required during download.

    Args:
        model_id: HuggingFace repo id (e.g., "XiaomiMiMo/MiMo-V2.5-ASR")
        hf_token: Optional HuggingFace token for gated models
        cache_dir: Optional unified cache directory. When set, downloads to
                   <cache_dir>/mimo-v2.5-asr/ instead of the HF cache.

    Returns:
        Path to the cached model directory

    Raises:
        ImportError: If huggingface_hub is not installed
        RuntimeError: If download fails
    """
    try:
        from huggingface_hub import snapshot_download
    except ImportError:
        raise ImportError("huggingface_hub not found. Install with: pip install huggingface-hub") from None

    emit_progress("starting", 0.0)

    try:
        if hf_token:
            import os
            os.environ["HF_TOKEN"] = hf_token

        local_dir = Path(cache_dir) / "mimo-v2.5-asr" if cache_dir else None
        if local_dir:
            local_dir.mkdir(parents=True, exist_ok=True)

        emit_progress("downloading", 0.3)

        snapshot_download(
            repo_id=model_id,
            local_dir=str(local_dir) if local_dir else None,
            token=hf_token,
            resume_download=True,
        )

        emit_progress("completed", 1.0)

        if local_dir:
            return str(local_dir)
        else:
            # Determine HF cache path for backwards compat
            cache_base = Path.home() / ".cache" / "huggingface" / "hub"
            model_name = model_id.replace("/", "--")
            return str(cache_base / f"models--{model_name}")

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
        "--cache-dir",
        type=str,
        default="~/.meetnote/models",
        help="Unified model cache directory (default: ~/.meetnote/models)"
    )

    args = parser.parse_args()

    hf_token = sys.stdin.readline().strip() or None

    # Resolve cache dir
    cache_dir = str(Path(args.cache_dir).expanduser().resolve())

    try:
        if args.backend == "paraformer":
            cache_path = download_paraformer(args.model_id, hf_token, cache_dir)
        elif args.backend == "qwen3-asr":
            cache_path = download_qwen3_asr(args.model_id, hf_token, cache_dir)
        elif args.backend == "mimo":
            cache_path = download_mimo(args.model_id, hf_token, cache_dir)
        else:
            raise ValueError(f"Unknown backend: {args.backend}")

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
