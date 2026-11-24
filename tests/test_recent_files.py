# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""File ▸ Open Recent: the last documents, remembered across sessions,
newest first, missing ones dropped, imports never listed."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QSettings  # noqa: E402
from PySide6.QtWidgets import QApplication  # noqa: E402

if QApplication.instance() is None:
    QApplication([])


def _reset():
    for key in ("home/recent_files", "recent_files", "home/last_file", "home/meta"):
        QSettings().remove(key)


def test_old_recents_are_read_once_and_clearing_stays_cleared(tmp_path):
    """The plain path list older builds kept is the list until the first
    write, which drops it; Clear list then cannot bring it back."""
    from core import recent
    _reset()
    old, pinned, new = (tmp_path / f"{n}.igz" for n in ("old", "pinned", "new"))
    for p in (old, pinned, new):
        p.write_text("{}")
    QSettings().setValue("recent_files", [str(old), str(pinned)])
    assert [r["path"] for r in recent.entries()] == [str(old), str(pinned)]
    assert recent.menu() == [str(old), str(pinned)]
    recent.remember(pinned, pinned=True)
    recent.remember(new)
    assert not QSettings().contains("recent_files")
    assert recent.menu() == [str(pinned.resolve()), str(new.resolve()), str(old)]
    recent.clear()
    assert recent.menu() == [str(pinned.resolve())]                # a pin stays
    assert not QSettings().value("home/last_file", "")
    QSettings().setValue("recent_files", [str(old)])                # an older build wrote it
    assert recent.menu() == [str(pinned.resolve())]
    _reset()


def test_recent_list_is_ordered_deduplicated_and_capped(tmp_path):
    from views.main_window import MainWindow
    win = MainWindow()
    try:
        _reset()
        docs = []
        for i in range(12):
            p = tmp_path / f"doc{i}.igz"
            p.write_text("{}")
            docs.append(p)
            win._remember_recent(p)
        recent = win._recent_paths()
        assert len(recent) == MainWindow._RECENT_MAX
        assert recent[0] == str(docs[-1].resolve())          # newest first
        win._remember_recent(docs[-3])                        # re-open an old one
        assert win._recent_paths()[0] == str(docs[-3].resolve())
        assert len(win._recent_paths()) == MainWindow._RECENT_MAX
        win._remember_recent(tmp_path / "modelo.skp")         # an import
        assert not any(x.endswith(".skp") for x in win._recent_paths())
        # A file deleted meanwhile is not offered.
        docs[-1].unlink()
        win._fill_recent_menu()
        labels = [a.text() for a in win._recent_menu.actions() if a.text()]
        assert not any("doc11.igz" in t for t in labels)
        assert any("doc9.igz" in t for t in labels)
        assert any("Clear" in t or "Limpiar" in t for t in labels)
    finally:
        _reset()
        win._saved_version = win.viewport.scene.version
        win.close()
