"""Read the Assistant provider, model and API key from MeetNote's v2 settings."""
from __future__ import annotations

import json
from pathlib import Path

LOOPBACK_HTTP = tuple(
    f"http://{host}{sep}" for host in ("localhost", "127.0.0.1", "[::1]") for sep in (":", "/")
)


def load_provider(settings_path: Path, provider_id: str | None) -> tuple[str, str, str]:
    """Return (chat base URL, model, apiKey) for a provider; defaults to assistant.model.

    The base URL is the provider's `endpoints.chat` (OpenAI Chat Completions, version
    prefix included). Custom providers store it there; built-in vendors only carry an
    optional override, since their real URL lives in the Rust catalog. HTTPS is required
    unless the host is loopback (localhost, 127.0.0.1, [::1]).
    """
    settings = json.loads(settings_path.read_text(encoding="utf-8"))
    assistant = settings["assistant"]
    choice = assistant.get("model") or {}
    provider_id = provider_id or choice.get("provider")
    if not provider_id:
        raise RuntimeError("no provider given and assistant.model is not set")
    provider = next((p for p in assistant["providers"] if p.get("id") == provider_id), None)
    if provider is None:
        raise RuntimeError(f"provider {provider_id} is not in settings.json")
    base_url = (provider.get("endpoints") or {}).get("chat")
    if not isinstance(base_url, str) or not base_url:
        raise RuntimeError(
            f"provider {provider_id} needs an endpoints.chat URL: a custom endpoint, "
            "or an override in the Providers page's Advanced section"
        )
    if not (base_url.startswith("https://") or base_url.startswith(LOOPBACK_HTTP)):
        raise RuntimeError(
            f"provider {provider_id} endpoints.chat must be HTTPS (http only for loopback)"
        )
    model = choice.get("model") if choice.get("provider") == provider_id else None
    if not isinstance(model, str) or not model:
        raise RuntimeError(f"provider {provider_id} is not the selected assistant.model")
    credentials = json.loads(
        (settings_path.parent / "credentials.json").read_text(encoding="utf-8")
    )
    credential = credentials.get("providers", {}).get(provider_id) or {}
    key = credential.get("key") if credential.get("type") == "apiKey" else None
    if not isinstance(key, str) or not key:
        raise RuntimeError(f"provider {provider_id} has no API key in credentials.json")
    return base_url.rstrip("/"), model, key
