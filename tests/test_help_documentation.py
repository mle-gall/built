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
    assert opened == ["https://github.com/mle-gall/built/tree/main/docs"]


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


def _user_visible_sources():
    """Where text the app shows comes from: the Rust UI and core sources,
    the menus, the bundled extensions, translations and resources."""
    root = Path(__file__).resolve().parent.parent
    for folder, patterns in [("rust/built_render/src", ("*.rs", "*.json")),
                             ("rust/built_core/src", ("*.rs", "*.json")),
                             ("rust/built_app/src", ("*.rs",)),
                             ("plugins", ("*.py", "*.json")),
                             ("i18n", ("*.json",)),
                             ("resources", ("*.json", "*.md", "*.xml", "*.txt", "*.html"))]:
        for pattern in patterns:
            yield from (root / folder).rglob(pattern)


def test_no_user_visible_text_names_the_old_site():
    """Built's links go to Built's own repository and docs (P0 Built links)."""
    found = [str(p) for p in _user_visible_sources()
             if "ingetrazo.com" in p.read_text(encoding="utf-8", errors="ignore").lower()]
    assert not found


def test_extension_and_help_links_point_at_built():
    root = Path(__file__).resolve().parent.parent
    menu = (root / "rust/built_render/src/model_menu.rs").read_text(encoding="utf-8")
    develop = menu[menu.index('"develop_plugin"'):]
    develop = develop[:develop.index(");")]
    assert "https://github.com/mle-gall/built/blob/main/docs/plugins.md" in develop
    assert (root / "docs/plugins.md").is_file()
    commands = (root / "rust/built_render/src/window_commands.rs").read_text(encoding="utf-8")
    assert '"https://github.com/mle-gall/built/tree/main/docs"' in commands
