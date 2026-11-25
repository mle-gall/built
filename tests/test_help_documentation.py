# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Help ▸ Documentation opens the online manual (#1036), at the address the
manual is really published on, and its texts exist in every language."""
import json
from pathlib import Path

from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_help_documentation_opens_the_manual(native_host, monkeypatch):
    opened = []
    monkeypatch.setattr("built_core.desktop_open.open_url", opened.append)
    action = native_host.window._registry_actions["menu:documentation"]
    action.trigger()
    assert opened == ["https://ingetrazo-docs.pages.dev/"]


def test_documentation_texts_translated_everywhere():
    keys = ("Documentation",
            "Open the Built documentation in the web browser.")
    root = Path(__file__).resolve().parent.parent / "i18n"
    for f in sorted(root.glob("*.json")):
        if f.stem == "en":
            continue
        data = json.loads(f.read_text(encoding="utf-8"))
        for k in keys:
            assert data.get(k), f"{f.name}: {k!r}"
