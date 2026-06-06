"""Device resolution for PyTorch inference."""

import sys
import json


def get_device(requested: str) -> str:
    """Resolve requested device to an actual available device."""
    import torch
    if requested == "auto":
        if torch.cuda.is_available():
            return "cuda"
        if torch.backends.mps.is_available():
            return "mps"
        return "cpu"
    elif requested == "mps":
        if not torch.backends.mps.is_available():
            print(json.dumps({"error": "MPS not available on this system"}))
            sys.exit(1)
        return "mps"
    elif requested == "cuda":
        if not torch.cuda.is_available():
            print(json.dumps({"error": "CUDA not available on this system"}))
            sys.exit(1)
        return "cuda"
    return "cpu"
