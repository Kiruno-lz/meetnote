#!/usr/bin/env python3

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from eval_settings import load_provider


def provider(pid, chat=None):
    return {
        "id": pid,
        "enabled": True,
        "region": None,
        "custom": None,
        "endpoints": {"chat": chat, "responses": None, "messages": None},
        "models": {},
    }


class LoadProviderTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.dir = Path(self._tmp.name)

    def write(self, providers, model=None, keys=None):
        settings = {
            "schema": "meetnote.settings.v2",
            "assistant": {"providers": providers, "model": model},
        }
        (self.dir / "settings.json").write_text(json.dumps(settings), encoding="utf-8")
        keys = {"gw": {"type": "apiKey", "key": "sk-1"}} if keys is None else keys
        (self.dir / "credentials.json").write_text(
            json.dumps({"providers": keys}), encoding="utf-8"
        )
        return self.dir / "settings.json"

    def test_custom_provider_uses_chat_endpoint(self):
        path = self.write(
            [provider("gw", "https://gw.example.com/v1/")],
            {"provider": "gw", "model": "m1", "protocol": "openaiChatCompletions"},
        )
        self.assertEqual(
            load_provider(path, "gw"), ("https://gw.example.com/v1", "m1", "sk-1")
        )

    def test_default_provider_comes_from_assistant_model(self):
        path = self.write(
            [provider("other", "https://o.example.com/v1"), provider("gw", "https://gw.example.com/v1")],
            {"provider": "gw", "model": "m1"},
        )
        self.assertEqual(load_provider(path, None)[0], "https://gw.example.com/v1")

    def test_builtin_without_chat_url_raises(self):
        path = self.write([provider("gw")], {"provider": "gw", "model": "m1"})
        with self.assertRaisesRegex(RuntimeError, "endpoints.chat"):
            load_provider(path, "gw")

    def test_non_https_remote_raises(self):
        path = self.write(
            [provider("gw", "http://gw.example.com/v1")], {"provider": "gw", "model": "m1"}
        )
        with self.assertRaisesRegex(RuntimeError, "HTTPS"):
            load_provider(path, "gw")

    def test_loopback_http_allowed(self):
        for url in ("http://localhost:4000/v1", "http://127.0.0.1:4000/v1", "http://[::1]:4000/v1"):
            path = self.write([provider("gw", url)], {"provider": "gw", "model": "m1"})
            self.assertEqual(load_provider(path, "gw")[0], url)

    def test_lookalike_host_rejected(self):
        path = self.write(
            [provider("gw", "http://localhost.evil.com/v1")], {"provider": "gw", "model": "m1"}
        )
        with self.assertRaisesRegex(RuntimeError, "HTTPS"):
            load_provider(path, "gw")

    def test_missing_key_raises(self):
        path = self.write(
            [provider("gw", "https://gw.example.com/v1")],
            {"provider": "gw", "model": "m1"},
            keys={},
        )
        with self.assertRaisesRegex(RuntimeError, "API key"):
            load_provider(path, "gw")


if __name__ == "__main__":
    unittest.main()
