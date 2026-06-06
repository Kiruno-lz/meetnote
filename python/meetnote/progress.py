"""Progress reporting to stderr for Python CLI scripts."""

import sys


def emit_progress(stage: str) -> None:
    """Emit a PROGRESS: line to stderr and flush."""
    print(f"PROGRESS:{stage}", file=sys.stderr, flush=True)
