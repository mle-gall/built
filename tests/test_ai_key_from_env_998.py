# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""#998: a user who keeps ``ANTHROPIC_API_KEY`` (and the other providers'
keys) in the environment does not have to paste it again. With the field
empty, a provider picked by name falls back to its usual variable, says so
under the field, and never copies it into the settings."""
from __future__ import annotations

import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401

from core import ai  # noqa: E402
from plugins.ai_assistant import AsistenteDialog, env_key_note  # noqa: E402


@pytest.fixture
def clean_env(monkeypatch):
    """No provider variable from the machine running the tests leaks in."""
    for names in ai.ENV_KEYS.values():
        for var in names:
            monkeypatch.delenv(var, raising=False)
    return monkeypatch


def test_env_key_reads_the_providers_usual_variable():
    env = {"ANTHROPIC_API_KEY": " sk-ant-env \n", "OPENAI_API_KEY": "   "}
    assert ai.env_key("anthropic", env) == ("ANTHROPIC_API_KEY", "sk-ant-env")
    assert ai.env_key("openai", env) is None            # blanks are unset
    assert ai.env_key("ollama", env) is None            # local: no key
    assert ai.env_key("gemini", {"GOOGLE_API_KEY": "AIzaG"}) \
        == ("GOOGLE_API_KEY", "AIzaG")
    assert ai.env_key("gemini", {"GOOGLE_API_KEY": "AIzaG",
                                 "GEMINI_API_KEY": "AIzaM"}) \
        == ("GEMINI_API_KEY", "AIzaM")                  # first one wins
    # Every hosted provider has its variable; the local one has none.
    assert set(ai.ENV_KEYS) == set(ai.PROVIDERS) - {"ollama"}


def test_note_under_the_field():
    env = {"ANTHROPIC_API_KEY": "sk-ant-env"}
    assert "ANTHROPIC_API_KEY" in env_key_note("anthropic", "", env)
    assert env_key_note("anthropic", "sk-ant-typed", env) == ""
    assert env_key_note("groq", "", env) == ""
    # On Auto an empty field is the local server: the note only points at
    # the provider to pick.
    note = env_key_note("auto", "", env)
    assert "ANTHROPIC_API_KEY" in note
    assert ai.PROVIDER_INFO["anthropic"][0] in note
    assert env_key_note("auto", "", {}) == ""


def test_placeholder_says_what_an_empty_field_means():
    """«empty = local AI» showed with Anthropic picked, where an empty
    field is the environment's key or no key at all."""
    from plugins.ai_assistant import key_placeholder
    local = key_placeholder("auto", {})
    assert "Ollama" in local
    assert key_placeholder("ollama", {}) == local
    env = {"ANTHROPIC_API_KEY": "sk-ant-env"}
    assert "ANTHROPIC_API_KEY" in key_placeholder("anthropic", env)
    named = key_placeholder("anthropic", {})
    assert "Ollama" not in named
    assert ai.PROVIDER_INFO["anthropic"][0] in named


def test_key_source_is_readable_in_the_native_settings_panel(native_host):
    panel = _panel(native_host)
    panel.event("provider", "commit", "anthropic")
    items = panel.sections()[0]["items"]
    source = next(row for row in items if row["id"] == "key_source")
    assert source["kind"] == "log" and "console.anthropic.com" in source["lines"][0]


def _panel(host):
    from plugins.ai_assistant import create_panel
    from views.extension_api import ExtensionApp
    return create_panel(ExtensionApp(host.window, "ai_assistant"), host.viewport.update)


def test_named_provider_uses_the_environment_key(clean_env, native_host):
    clean_env.setenv("ANTHROPIC_API_KEY", "sk-ant-from-env")
    panel = _panel(native_host)
    panel.event("provider", "commit", "anthropic")
    panel.key = ""
    assert "ANTHROPIC_API_KEY" in str(panel.sections())
    assert panel._config()[2] == "sk-ant-from-env"
    seen = []
    clean_env.setattr(ai, "probar_conexion", lambda p, m, k, **kw: (seen.append(k) or True, "OK"))
    panel.event("test", "invoke", None)
    from tests.test_ai_assistant import wait_reply
    wait_reply(panel)
    assert seen == ["sk-ant-from-env"]
    assert not panel.store.value("ia/api_key")
    assert panel.store.value("ia/claves/anthropic") != "sk-ant-from-env"
    panel.key = "sk-ant-typed"
    assert panel._config()[2] == panel.key


def test_auto_with_an_empty_field_stays_local(clean_env, native_host):
    clean_env.setenv("OPENAI_API_KEY", "sk-env")
    panel = _panel(native_host)
    panel.event("provider", "commit", "auto")
    panel.key = ""
    provider, _, key, _ = panel._config()
    assert (provider, key) == ("ollama", "")
    assert "OPENAI_API_KEY" in str(panel.sections())


def test_missing_key_hint_names_the_variable(clean_env):
    hint = AsistenteDialog._missing_key_hint(None, "anthropic", "")
    assert "ANTHROPIC_API_KEY" in hint
    assert "GEMINI_API_KEY" in AsistenteDialog._missing_key_hint(
        None, "gemini", "")


def test_key_note_has_one_owner(clean_env, native_host):
    from plugins.ai_assistant import env_key_note
    from tests.preferences_chrome import Controls
    clean_env.setenv("GROQ_API_KEY", "gsk_env")
    assert "GROQ_API_KEY" in env_key_note("groq", "")
    assert not env_key_note("groq", "gsk_typed")
    settings = Controls(native_host.window, "ai")
    assert "ai" in {row.get("id") for row in settings.wire()["side"]}
    assert settings.value("ia/api_key") == ""
    assert "GROQ_API_KEY" not in str(settings.wire())
