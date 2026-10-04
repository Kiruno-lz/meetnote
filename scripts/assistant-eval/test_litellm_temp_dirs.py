#!/usr/bin/env python3

import sys
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

import litellm_deepseek_smoke
import run_matrix_litellm


class FakeProcess:
    def __init__(self, command, **_kwargs):
        self.config = Path(command[command.index("--config") + 1])

    def terminate(self):
        pass

    def wait(self, timeout=None):
        return 0

    def kill(self):
        pass


class GatewayStopped(RuntimeError):
    pass


def stop_gateway(_port, _process):
    raise GatewayStopped


class LiteLLMTemporaryDirectoryTests(unittest.TestCase):
    """The gateway config lives under eval-data/ and works without target/."""

    def assert_config_created_and_removed(self, module, root: Path, invoke, prefix: str) -> None:
        started: list[FakeProcess] = []

        def popen(command, **kwargs):
            process = FakeProcess(command, **kwargs)
            self.assertTrue(process.config.is_file())
            started.append(process)
            return process

        with patch.object(module, "ROOT", root), patch.object(
            module, "load_connection", return_value=("http://127.0.0.1:9/v1", "model", "secret")
        ), patch.object(module, "free_port", return_value=9), patch.object(
            module.subprocess, "Popen", side_effect=popen
        ), patch.object(module, "wait_for_port", side_effect=stop_gateway):
            with self.assertRaises(GatewayStopped):
                invoke()

        self.assertEqual(len(started), 1)
        work = started[0].config.parent
        self.assertEqual(work.parent, root / "eval-data/assistant")
        self.assertTrue(work.name.startswith(prefix))
        self.assertFalse(work.exists())
        self.assertFalse((root / "target").exists())

    def test_smoke_creates_config_under_eval_data_without_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.assert_config_created_and_removed(
                litellm_deepseek_smoke,
                root,
                lambda: litellm_deepseek_smoke.smoke("litellm", None, None),
                "litellm-deepseek-smoke-",
            )

    def test_matrix_gateway_creates_config_under_eval_data_without_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = Namespace(
                model=None,
                provider_model=None,
                output_dir=root / "runs",
                writeback_root=None,
                binary="litellm",
                writeback_cli=None,
            )
            self.assert_config_created_and_removed(
                run_matrix_litellm,
                root,
                lambda: run_matrix_litellm.run(args),
                "litellm-",
            )


if __name__ == "__main__":
    unittest.main()
