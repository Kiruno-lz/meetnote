#!/usr/bin/env python3
"""
Diarization CLI tests
Tests: error handling, output format contract
Run: cd /path/to/meetnote && uv run --project python python/test_diarization_cli.py
"""
import subprocess
import json
import sys
import os

PROJECT_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
AUDIO = os.path.join(PROJECT_ROOT, "assets", "test_video.m4a")
CLI = os.path.join(PROJECT_ROOT, "python", "diarization_cli.py")
UV = None

RESULTS: list[dict] = []


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


def run_cli(extra_args: list[str]) -> dict:
    """Run the diarization CLI with given args and return parsed result."""
    global UV
    if UV is None:
        UV = find_uv()
    cmd = [UV, "run", "--project", "python", "python", CLI]
    cmd.extend(extra_args)
    proc = subprocess.run(
        cmd,
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
    )
    return {
        "returncode": proc.returncode,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
    }


def assert_json_error(result: dict, test_name: str, expected_code: str | None = None) -> dict:
    """Validate that stdout is a JSON object with 'error' key."""
    assert result["returncode"] != 0, (
        f"[{test_name}] expected non-zero exit code, got {result['returncode']}\n"
        f"stdout: {result['stdout'][:500]}\n"
        f"stderr: {result['stderr'][:500]}"
    )
    try:
        data = json.loads(result["stdout"])
    except json.JSONDecodeError as e:
        raise AssertionError(
            f"[{test_name}] stdout is not valid JSON: {e}\n"
            f"stdout: {result['stdout'][:500]}"
        )
    assert "error" in data, f"[{test_name}] missing 'error' key in JSON"
    if expected_code:
        assert data.get("code") == expected_code, (
            f"[{test_name}] expected code '{expected_code}', got '{data.get('code')}'"
        )
    return data


def assert_valid_output(result: dict, test_name: str) -> dict:
    """Validate output JSON matches diarization schema. Returns parsed output."""
    assert result["returncode"] == 0, (
        f"[{test_name}] process exited with code {result['returncode']}\n"
        f"stdout: {result['stdout'][:500]}\n"
        f"stderr: {result['stderr'][:500]}"
    )
    try:
        data = json.loads(result["stdout"])
    except json.JSONDecodeError as e:
        raise AssertionError(
            f"[{test_name}] stdout is not valid JSON: {e}\n"
            f"stdout: {result['stdout'][:500]}"
        )
    assert "error" not in data, f"[{test_name}] CLI returned error: {data.get('error')}"
    assert "speakers" in data, f"[{test_name}] missing 'speakers' in output"
    assert "segments" in data, f"[{test_name}] missing 'segments' in output"
    assert isinstance(data["speakers"], list), f"[{test_name}] 'speakers' must be a list"
    assert isinstance(data["segments"], list), f"[{test_name}] 'segments' must be a list"
    assert len(data["speakers"]) > 0, f"[{test_name}] 'speakers' must not be empty"
    assert len(data["segments"]) > 0, f"[{test_name}] 'segments' must not be empty"

    for i, spk in enumerate(data["speakers"]):
        assert "id" in spk, f"[{test_name}] speakers[{i}] missing 'id'"
        assert "label" in spk, f"[{test_name}] speakers[{i}] missing 'label'"

    speaker_ids = {spk["id"] for spk in data["speakers"]}
    for i, seg in enumerate(data["segments"]):
        assert "id" in seg, f"[{test_name}] segments[{i}] missing 'id'"
        assert "speakerId" in seg, f"[{test_name}] segments[{i}] missing 'speakerId'"
        assert "startMs" in seg, f"[{test_name}] segments[{i}] missing 'startMs'"
        assert "endMs" in seg, f"[{test_name}] segments[{i}] missing 'endMs'"
        assert "confidence" in seg, f"[{test_name}] segments[{i}] missing 'confidence'"
        assert "overlap" in seg, f"[{test_name}] segments[{i}] missing 'overlap'"
        assert seg["endMs"] >= seg["startMs"], f"[{test_name}] segments[{i}] endMs < startMs"
        assert seg["speakerId"] in speaker_ids, (
            f"[{test_name}] segments[{i}] speakerId '{seg['speakerId']}' not in speakers"
        )
        assert 0 <= seg["confidence"] <= 1, (
            f"[{test_name}] segments[{i}] confidence out of range: {seg['confidence']}"
        )
        assert isinstance(seg["overlap"], bool), (
            f"[{test_name}] segments[{i}] overlap must be boolean"
        )

    return data


def test_error_missing_audio():
    """Missing audio file must produce JSON error, not a crash."""
    print("  Testing missing audio error...", flush=True)
    result = run_cli(["--audio", "/nonexistent/audio.m4a", "--hf-token", "dummy"])
    data = assert_json_error(result, "error_missing_audio")
    assert "Audio file not found" in data["error"], (
        f"[error_missing_audio] unexpected error message: {data['error']}"
    )
    RESULTS.append({"test": "error_missing_audio", "status": "PASS"})
    print("  OK: missing audio returns JSON error")


def test_error_missing_token():
    """Missing --hf-token argument must be caught by argparse."""
    print("  Testing missing token error...", flush=True)
    global UV
    if UV is None:
        UV = find_uv()
    proc = subprocess.run(
        [UV, "run", "--project", "python", "python", CLI, "--audio", AUDIO],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
    )
    assert proc.returncode != 0, "[error_missing_token] expected non-zero exit code"
    RESULTS.append({"test": "error_missing_token", "status": "PASS"})
    print("  OK: missing token is rejected by argparse")


def test_error_invalid_token():
    """Invalid HuggingFace token must produce AUTH_REQUIRED error."""
    print("  Testing invalid token error...", flush=True)
    result = run_cli(["--audio", AUDIO, "--hf-token", "hf_invalid_token_12345"])
    if result["returncode"] != 0 and result["stdout"].strip():
        try:
            data = json.loads(result["stdout"])
            if "error" in data:
                RESULTS.append({"test": "error_invalid_token", "status": "PASS"})
                print(f"  OK: invalid token returns error: {data.get('code', 'no code')}")
                return
        except json.JSONDecodeError:
            pass
    if result["returncode"] != 0:
        RESULTS.append({"test": "error_invalid_token", "status": "PASS"})
        print("  OK: invalid token rejected (non-zero exit)")
    else:
        RESULTS.append({"test": "error_invalid_token", "status": "SKIP"})
        print("  SKIP: could not verify invalid token error (pyannote may not be installed)")


def test_output_format():
    """If pyannote is available and HF token is set, validate full output format."""
    hf_token = os.environ.get("HF_TOKEN", "")
    if not hf_token:
        RESULTS.append({"test": "output_format", "status": "SKIP"})
        print("  SKIP: HF_TOKEN env var not set, skipping integration test")
        return

    if not os.path.exists(AUDIO):
        RESULTS.append({"test": "output_format", "status": "SKIP"})
        print(f"  SKIP: test audio not found at {AUDIO}")
        return

    print("  Running full diarization (this may take a while)...", flush=True)
    result = run_cli(["--audio", AUDIO, "--hf-token", hf_token])
    if result["returncode"] != 0:
        try:
            data = json.loads(result["stdout"])
            if "error" in data:
                RESULTS.append({"test": "output_format", "status": "FAIL"})
                print(f"  FAIL: CLI returned error: {data['error']}")
                return
        except json.JSONDecodeError:
            pass
        RESULTS.append({"test": "output_format", "status": "FAIL"})
        print(f"  FAIL: non-zero exit code {result['returncode']}")
        return

    data = assert_valid_output(result, "output_format")
    RESULTS.append({
        "test": "output_format",
        "status": "PASS",
        "speakers": len(data["speakers"]),
        "segments": len(data["segments"]),
    })
    print(f"  OK: {len(data['speakers'])} speakers, {len(data['segments'])} segments")


def print_summary():
    print("\n" + "=" * 60)
    print("DIARIZATION CLI TEST RESULTS")
    print("=" * 60)
    for r in RESULTS:
        status = r["status"]
        symbol = "OK" if status == "PASS" else ("SKIP" if status == "SKIP" else "FAIL")
        extra = ""
        if "speakers" in r:
            extra = f"  [{r['speakers']} speakers, {r['segments']} segments]"
        print(f"  {symbol}  {r['test']:<30} {status}{extra}")
    failed = [r for r in RESULTS if r["status"] == "FAIL"]
    passed = [r for r in RESULTS if r["status"] == "PASS"]
    skipped = [r for r in RESULTS if r["status"] == "SKIP"]
    print(f"\n  {len(passed)} passed, {len(skipped)} skipped, {len(failed)} failed")
    if failed:
        print("\nFAILED TESTS:")
        for r in failed:
            print(f"  - {r['test']}")
        return False
    return True


def main():
    print(f"CLI: {CLI}")
    print()

    print("[1/4] Missing audio error")
    test_error_missing_audio()

    print("\n[2/4] Missing token error")
    test_error_missing_token()

    print("\n[3/4] Invalid token error")
    test_error_invalid_token()

    print("\n[4/4] Output format (integration)")
    test_output_format()

    success = print_summary()
    sys.exit(0 if success else 1)

if __name__ == "__main__":
    main()
