# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Outstanding bundled extension menu ordering and shortcut coverage."""
import pytest


def _close(w):
    w._saved_version = w.viewport.scene.version   # no "save?" modal
    w.close()


@pytest.fixture
def deferred():
    from views.main_window import MainWindow
    w = MainWindow(defer_extensions=True)
    yield w
    _close(w)


@pytest.mark.classic_pending
def test_the_python_console_comes_after_the_model_tools(deferred):
    # Marco, 09-10: Model Info, Solid Inspector, then the Python Console —
    # a tool for developers — not in between by file name.
    deferred.load_extensions()
    texts = _menu_texts(deferred)
    order = [texts.index(t) for t in ("Model Info", "Solid Inspector",
                                      "Python Console") if t in texts]
    assert order == sorted(order) and len(order) == 3, texts


@pytest.mark.classic_pending
def test_an_extension_cannot_take_a_bundled_tools_shortcut(tmp_path, monkeypatch):
    # Extension bench (10-10): Point Cloud Splat's setup() asked for
    # Ctrl+Shift+P before the Python Console's entry was made, and the
    # console lost its key. The tools that ship with IngeTrazo reserve theirs.
    from core import extensions
    from core.paths import app_root
    from views.main_window import MainWindow
    (tmp_path / "grabby.py").write_text(
        "def setup(app):\n"
        "    app.add_menu_action('Grabby', lambda: None, 'Ctrl+Shift+P')\n")
    monkeypatch.setattr(extensions, "plugin_dirs",
                        lambda: [app_root() / "plugins", tmp_path])
    w = MainWindow(defer_extensions=True)
    try:
        w.load_extensions()
        acts = {a.text(): a for a in w._ext_menu.actions()}
        assert acts["Python Console"].shortcut().toString() == "Ctrl+Shift+P"
        assert acts["Grabby"].shortcut().isEmpty()
    finally:
        _close(w)
