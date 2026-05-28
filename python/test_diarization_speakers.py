#!/usr/bin/env python3
"""
Diarization speaker differentiation test.
Validates that the diarization CLI correctly identifies ≥2 speakers
in assets/test_video.m4a (known to contain 2 speakers).

Run: cd /path/to/meetnote && uv run --project python python/test_diarization_speakers.py
Requires: HF_TOKEN in environment or .env file at project root.
"""
import subprocess
import json
import sys
import os

PROJECT_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
AUDIO = os.path.join(PROJECT_ROOT, "assets", "test_video.m4a")
CLI = os.path.join(PROJECT_ROOT, "python", "diarization_cli.py")
EXPECTED_MIN_SPEAKERS = 2


def find_uv():
    import shutil
    for candidate in [
        os.path.expanduser("~/.local/bin/uv"),
        "/opt/homebrew/bin/uv",
        "/usr/local/bin/uv",
        shutil.which("uv") or "",
    ]:
        if candidate and os.path.exists(candidate):
            return candidate
    raise RuntimeError("uv not found")


def load_hf_token() -> str:
    """Load HF_TOKEN from environment or .env file."""
    token = os.environ.get("HF_TOKEN", "").strip()
    if token:
        return token
    env_path = os.path.join(PROJECT_ROOT, ".env")
    if os.path.exists(env_path):
        with open(env_path) as f:
            for line in f:
                line = line.strip()
                if line.startswith("HF_TOKEN=") and not line.startswith("#"):
                    token = line[len("HF_TOKEN="):].strip().strip('"').strip("'")
                    if token:
                        return token
    return ""


def run_diarization(hf_token: str) -> dict:
    """Run the diarization CLI and return parsed result."""
    uv = find_uv()
    cmd = [
        uv, "run", "--project", "python", "python", CLI,
        "--audio", AUDIO,
        "--hf-token", hf_token,
    ]
    proc = subprocess.run(cmd, cwd=PROJECT_ROOT, capture_output=True, text=True, timeout=300)
    return {
        "returncode": proc.returncode,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
    }


def main():
    print("=" * 60)
    print("DIARIZATION SPEAKER DIFFERENTIATION TEST")
    print("=" * 60)
    print(f"Audio: {AUDIO}")
    print(f"CLI:   {CLI}")
    print()

    # Step 0: Check prerequisites
    if not os.path.exists(AUDIO):
        print(f"FAIL: test audio not found at {AUDIO}")
        sys.exit(1)

    hf_token = load_hf_token()
    if not hf_token:
        print("SKIP: HF_TOKEN not found in environment or .env file")
        print("      Set HF_TOKEN and re-run to execute this test.")
        sys.exit(0)

    print(f"HF token loaded ({len(hf_token)} chars)")

    # Step 1: Run diarization
    print("\nRunning diarization (this may take 1-3 minutes)...", flush=True)
    result = run_diarization(hf_token)

    # Step 2: Check exit code
    if result["returncode"] != 0:
        print(f"\nFAIL: CLI exited with code {result['returncode']}")
        print(f"stdout: {result['stdout'][:500]}")
        print(f"stderr: {result['stderr'][:500]}")
        # Try to parse error JSON
        try:
            data = json.loads(result["stdout"])
            if "error" in data:
                print(f"\nError: {data['error']}")
                if data.get("code"):
                    print(f"Code:  {data['code']}")
        except json.JSONDecodeError:
            pass
        sys.exit(1)

    # Step 3: Parse and validate output
    try:
        data = json.loads(result["stdout"])
    except json.JSONDecodeError as e:
        print(f"\nFAIL: stdout is not valid JSON: {e}")
        print(f"stdout: {result['stdout'][:500]}")
        sys.exit(1)

    if "error" in data:
        print(f"\nFAIL: CLI returned error: {data['error']}")
        sys.exit(1)

    # Step 4: Validate schema
    assert "speakers" in data, "missing 'speakers' key"
    assert "segments" in data, "missing 'segments' key"
    assert isinstance(data["speakers"], list), "'speakers' must be a list"
    assert isinstance(data["segments"], list), "'segments' must be a list"

    speakers = data["speakers"]
    segments = data["segments"]

    print(f"\nResults:")
    print(f"  Speakers: {len(speakers)}")
    print(f"  Segments: {len(segments)}")

    for spk in speakers:
        print(f"    - {spk['id']}: {spk['label']}")

    # Step 5: Assert speaker count
    if len(speakers) < EXPECTED_MIN_SPEAKERS:
        print(f"\nFAIL: expected ≥{EXPECTED_MIN_SPEAKERS} speakers, got {len(speakers)}")
        print("      Diarization did not differentiate speakers in test_video.m4a")
        sys.exit(1)

    print(f"\nPASS: identified {len(speakers)} speakers (expected ≥{EXPECTED_MIN_SPEAKERS})")

    # Step 6: Validate segment schema
    speaker_ids = {spk["id"] for spk in speakers}
    for i, seg in enumerate(segments):
        assert "id" in seg, f"segments[{i}] missing 'id'"
        assert "speakerId" in seg, f"segments[{i}] missing 'speakerId'"
        assert "startMs" in seg, f"segments[{i}] missing 'startMs'"
        assert "endMs" in seg, f"segments[{i}] missing 'endMs'"
        assert seg["endMs"] >= seg["startMs"], f"segments[{i}] endMs < startMs"
        assert seg["speakerId"] in speaker_ids, (
            f"segments[{i}] speakerId '{seg['speakerId']}' not in speakers"
        )

    print("PASS: all segment schemas valid")
    print("\n" + "=" * 60)
    print("ALL CHECKS PASSED")
    print("=" * 60)


if __name__ == "__main__":
    main()
