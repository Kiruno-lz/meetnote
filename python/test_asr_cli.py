#!/usr/bin/env python3
"""
ASR CLI 集成测试
Tests: output format contract, MPS acceleration, CPU multi-threading
Run: cd /path/to/meetnote && uv run --project python python/test_asr_cli.py
"""
import subprocess
import json
import sys
import time
import os

PROJECT_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
AUDIO = os.path.join(PROJECT_ROOT, "assets", "test_video.m4a")
CLI = os.path.join(PROJECT_ROOT, "python", "asr_cli.py")
UV = None  # resolved at runtime

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

def run_cli(device: str, model: str = "tiny", extra_args: list[str] | None = None) -> dict:
    """Run the ASR CLI and return parsed stdout JSON + timing info."""
    global UV
    if UV is None:
        UV = find_uv()
    cmd = [UV, "run", "--project", "python", "python", CLI,
           "--audio", AUDIO, "--model", model, "--language", "zh", "--device", device]
    if extra_args:
        cmd.extend(extra_args)
    t0 = time.time()
    proc = subprocess.run(
        cmd,
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
    )
    elapsed = time.time() - t0
    return {
        "returncode": proc.returncode,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
        "elapsed": round(elapsed, 2),
        "device": device,
    }

def assert_valid_output(result: dict, test_name: str) -> dict:
    """Validate output JSON contract. Returns parsed output."""
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
    assert "segments" in data, f"[{test_name}] missing 'segments' in output"
    assert isinstance(data["segments"], list), f"[{test_name}] 'segments' must be a list"
    assert "language" in data, f"[{test_name}] missing 'language' in output"
    assert "model" in data, f"[{test_name}] missing 'model' in output"
    for i, seg in enumerate(data["segments"]):
        assert "id" in seg, f"[{test_name}] segments[{i}] missing 'id'"
        assert "startMs" in seg, f"[{test_name}] segments[{i}] missing 'startMs'"
        assert "endMs" in seg, f"[{test_name}] segments[{i}] missing 'endMs'"
        assert "text" in seg, f"[{test_name}] segments[{i}] missing 'text'"
        assert seg["endMs"] >= seg["startMs"], f"[{test_name}] segments[{i}] endMs < startMs"
    return data

def test_cpu_baseline():
    """BASELINE: CPU mode must produce valid output — gate for all other tests."""
    print("  Running CPU baseline...", flush=True)
    result = run_cli("cpu")
    data = assert_valid_output(result, "cpu_baseline")
    assert len(data["segments"]) > 0, "[cpu_baseline] no segments transcribed"
    RESULTS.append({"test": "cpu_baseline", "device": "cpu", "elapsed": result["elapsed"],
                     "segments": len(data["segments"]), "status": "PASS"})
    print(f"  ✓ CPU baseline: {len(data['segments'])} segments in {result['elapsed']}s")
    return result["elapsed"]

def test_mps():
    """MPS (Metal GPU) must load and transcribe successfully on Apple Silicon."""
    import torch  # type: ignore
    if not torch.backends.mps.is_available():
        print("  ⚠  MPS not available on this system — SKIP")
        RESULTS.append({"test": "mps", "device": "mps", "elapsed": 0, "segments": 0, "status": "SKIP"})
        return None
    print("  Running MPS...", flush=True)
    result = run_cli("mps")
    data = assert_valid_output(result, "mps")
    assert len(data["segments"]) > 0, "[mps] no segments transcribed"
    RESULTS.append({"test": "mps", "device": "mps", "elapsed": result["elapsed"],
                     "segments": len(data["segments"]), "status": "PASS"})
    print(f"  ✓ MPS: {len(data['segments'])} segments in {result['elapsed']}s")
    return result["elapsed"]

def test_cpu_multithreaded():
    """CPU multithreaded must produce identical segment count to baseline."""
    print("  Running CPU multithreaded...", flush=True)
    result = run_cli("cpu_multithreaded")
    data = assert_valid_output(result, "cpu_multithreaded")
    assert len(data["segments"]) > 0, "[cpu_multithreaded] no segments transcribed"
    RESULTS.append({"test": "cpu_multithreaded", "device": "cpu_multithreaded",
                     "elapsed": result["elapsed"], "segments": len(data["segments"]), "status": "PASS"})
    print(f"  ✓ CPU multithreaded: {len(data['segments'])} segments in {result['elapsed']}s")
    return result["elapsed"]

def test_error_missing_audio():
    """Missing audio file must produce JSON error, not a crash."""
    result = run_cli("cpu")
    # Override audio path to non-existent file
    import shutil
    uv = find_uv()
    proc = subprocess.run(
        [uv, "run", "--project", "python", "python", CLI,
         "--audio", "/nonexistent/audio.m4a", "--model", "tiny", "--language", "zh", "--device", "cpu"],
        cwd=PROJECT_ROOT, capture_output=True, text=True
    )
    assert proc.returncode != 0, "[error_missing_audio] should have non-zero exit code"
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError:
        raise AssertionError(f"[error_missing_audio] stdout is not valid JSON: {proc.stdout[:200]}")
    assert "error" in data, "[error_missing_audio] missing 'error' key in JSON"
    RESULTS.append({"test": "error_missing_audio", "device": "cpu", "elapsed": 0, "segments": 0, "status": "PASS"})
    print("  ✓ Error handling: missing audio returns JSON error")

def print_summary():
    print("\n" + "="*60)
    print("ASR CLI TEST RESULTS")
    print("="*60)
    for r in RESULTS:
        status = r["status"]
        symbol = "✓" if status == "PASS" else ("⊘" if status == "SKIP" else "✗")
        extra = f"  [{r['segments']} segs, {r['elapsed']}s]" if r["elapsed"] > 0 else ""
        print(f"  {symbol} {r['test']:<25} {status}{extra}")
    failed = [r for r in RESULTS if r["status"] == "FAIL"]
    passed = [r for r in RESULTS if r["status"] == "PASS"]
    print(f"\n  {len(passed)} passed, {len(failed)} failed")
    if failed:
        print("\nFAILED TESTS:")
        for r in failed:
            print(f"  - {r['test']}")
        return False
    return True

def main():
    if not os.path.exists(AUDIO):
        print(f"ERROR: Test audio not found: {AUDIO}")
        sys.exit(1)

    print(f"Test audio: {AUDIO}")
    print(f"CLI: {CLI}")
    print()

    print("[1/4] CPU baseline (gate test)")
    baseline_time = test_cpu_baseline()

    print("\n[2/4] MPS (Metal GPU)")
    mps_time = test_mps()
    if mps_time:
        speedup = round(baseline_time / mps_time, 2) if mps_time > 0 else "N/A"
        print(f"      Speedup vs CPU: {speedup}x")

    print("\n[3/4] CPU multithreaded")
    mt_time = test_cpu_multithreaded()
    if mt_time and baseline_time:
        speedup = round(baseline_time / mt_time, 2) if mt_time > 0 else "N/A"
        print(f"      Speedup vs single CPU: {speedup}x")

    print("\n[4/4] Error handling")
    test_error_missing_audio()

    success = print_summary()
    sys.exit(0 if success else 1)

if __name__ == "__main__":
    main()
