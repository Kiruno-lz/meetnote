#!/usr/bin/env python3
"""Run the complete Assistant matrix through a temporary real DeepSeek gateway.

The local Settings API key is read only at runtime and is injected into the
LiteLLM child environment. The generated config uses an environment reference
and is removed before this process exits. Matrix artifacts are kept under
eval-data/ and are checked for accidental credential persistence.
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
SETTINGS = ROOT / "data/settings/settings.json"
MATRIX_SCRIPT = ROOT / "scripts/assistant-eval/run_matrix.py"
RUNNER_SCRIPT = ROOT / "scripts/assistant-eval/runner.py"
DEFAULT_OUTPUT = ROOT / "eval-data/assistant/runs-deepseek-chat"
DEFAULT_WRITEBACK = ROOT / "eval-data/assistant/writeback-deepseek-chat"
PROTOCOLS = ("openaiChatCompletions", "anthropicMessages", "openaiResponses")
CASES = (
    "ask_direct",
    "ask_second_read",
    "ask_unknown",
    "organize_automatic",
    "organize_confirm_each",
    "validator_correction",
    "partition",
    "extract",
    "none",
    "nested_second_pass",
)


def free_port() -> int:
    with socket.socket() as server:
        server.bind(("127.0.0.1", 0))
        return int(server.getsockname()[1])


def load_connection() -> tuple[str, str, str]:
    settings = json.loads(SETTINGS.read_text(encoding="utf-8"))
    connection = next(
        item
        for item in settings["assistant"]["connections"]
        if item.get("id") == "local-litellm"
    )
    base_url = connection.get("baseUrl")
    model = connection.get("model")
    api_key = connection.get("apiKey")
    if not isinstance(base_url, str) or not base_url.startswith("https://"):
        raise RuntimeError("local-litellm connection must use an HTTPS upstream")
    if not isinstance(model, str) or not model:
        raise RuntimeError("local-litellm connection has no model")
    if not isinstance(api_key, str) or not api_key:
        raise RuntimeError("local-litellm connection has no API key")
    return base_url.rstrip("/"), model, api_key


def wait_for_port(port: int, process: subprocess.Popen[bytes]) -> None:
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("LiteLLM exited before opening its local port")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return
        except OSError:
            time.sleep(1)
    raise RuntimeError("LiteLLM did not open its local port within 90 seconds")


def write_config(path: Path, upstream: str, model: str, provider_model: str) -> None:
    path.write_text(
        "\n".join(
            [
                "model_list:",
                f"  - model_name: {model}",
                "    litellm_params:",
                f"      model: {provider_model}",
                f"      api_base: {upstream}",
                "      api_key: os.environ/DEEPSEEK_API_KEY",
                "general_settings:",
                "  master_key: local-matrix-key",
                "  disable_spend_logs: true",
                "litellm_settings:",
                "  telemetry: false",
                "",
            ]
        ),
        encoding="utf-8",
    )


def run_matrix(
    *,
    port: int,
    model: str,
    output_dir: Path,
    writeback_cli: str | None,
    writeback_root: Path | None,
) -> tuple[int, dict[str, Any]]:
    endpoint = f"http://127.0.0.1:{port}/v1"
    environment = dict(os.environ)
    environment.update(
        {
            "MEETNOTE_ASSISTANT_EVAL_MODEL": model,
            "MEETNOTE_ASSISTANT_EVAL_API_KEY": "local-matrix-key",
            "MEETNOTE_ASSISTANT_EVAL_AUTH_MODE": "bearer",
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_BASE_URL": endpoint,
            "MEETNOTE_ASSISTANT_EVAL_ANTHROPIC_MESSAGES_BASE_URL": endpoint,
            "MEETNOTE_ASSISTANT_EVAL_OPENAI_RESPONSES_BASE_URL": endpoint,
            "MEETNOTE_ASSISTANT_EVAL_DISABLE_THINKING": "1",
            "MEETNOTE_ASSISTANT_EVAL_RESPONSES_PROVIDER": "deepseek",
        }
    )
    runner = f"{shlex.quote(sys.executable)} {shlex.quote(str(RUNNER_SCRIPT))}"
    command = [
        sys.executable,
        str(MATRIX_SCRIPT),
        "--runner",
        runner,
        "--output-dir",
        str(output_dir),
        "--require-ready",
    ]
    if writeback_cli:
        command.extend(["--writeback-cli", writeback_cli])
    if writeback_root:
        command.extend(["--writeback-package-root", str(writeback_root)])
    completed = subprocess.run(
        command,
        cwd=ROOT,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    try:
        report = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("run_matrix did not produce a JSON report") from error
    return completed.returncode, report


def self_check(output_dir: Path, report: dict[str, Any], secret: str) -> dict[str, Any]:
    expected = {f"{protocol}/{case}" for protocol in PROTOCOLS for case in CASES}
    actual: set[str] = set()
    secret_paths: list[str] = []
    incomplete: list[str] = []
    for metadata in output_dir.glob("*/**/metadata.json"):
        relative = metadata.parent.relative_to(output_dir)
        actual.add(str(relative))
        receipt = metadata.parent / "receipt.json"
        readback = metadata.parent / "readback.json"
        if not receipt.is_file() or not readback.is_file():
            incomplete.append(str(relative))
        for artifact in (metadata, receipt, readback, metadata.parent / "prediction.jsonl"):
            if artifact.is_file() and secret.encode("utf-8") in artifact.read_bytes():
                secret_paths.append(str(artifact))
    return {
        "expectedRunCount": len(expected),
        "observedRunCount": len(actual),
        "allRunDirectoriesPresent": actual == expected,
        "artifactTripletsComplete": not incomplete,
        "secretPersisted": bool(secret_paths),
        "incompleteRuns": incomplete,
        "secretPaths": secret_paths,
        "statusCounts": report.get("statuses", {}),
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    upstream, configured_model, secret = load_connection()
    model = args.model or configured_model
    provider_model = args.provider_model or f"deepseek/{model}"
    output_dir = args.output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    writeback_root = args.writeback_root.resolve() if args.writeback_root else None
    port = free_port()
    temporary_root = ROOT / "eval-data/assistant"
    temporary_root.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix="litellm-", dir=temporary_root))
    config = temporary / "config.yaml"
    write_config(config, upstream, model, provider_model)
    gateway_env = dict(os.environ)
    gateway_env["DEEPSEEK_API_KEY"] = secret
    process = subprocess.Popen(
        [args.binary, "--config", str(config), "--host", "127.0.0.1", "--port", str(port)],
        cwd=ROOT,
        env=gateway_env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        wait_for_port(port, process)
        return_code, report = run_matrix(
            port=port,
            model=model,
            output_dir=output_dir,
            writeback_cli=args.writeback_cli,
            writeback_root=writeback_root,
        )
        check = self_check(output_dir, report, secret)
        result = {
            "status": "passed" if return_code == 0 and not check["secretPersisted"] else "failed",
            "model": model,
            "providerModel": provider_model,
            "upstream": upstream,
            "runMatrixExitCode": return_code,
            "outputDir": str(output_dir),
            "report": report,
            "selfCheck": check,
        }
        (output_dir / "gateway-run-report.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return result
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        shutil.rmtree(temporary, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="/opt/miniconda3/envs/default/bin/litellm")
    parser.add_argument("--model", default="deepseek-chat")
    parser.add_argument("--provider-model", default="deepseek/deepseek-chat")
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--writeback-cli")
    parser.add_argument("--writeback-root", type=Path, default=DEFAULT_WRITEBACK)
    args = parser.parse_args()
    if not Path(args.binary).is_file():
        raise SystemExit("LiteLLM executable is missing")
    result = run(args)
    print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if result["status"] == "passed" else 2


if __name__ == "__main__":
    raise SystemExit(main())
