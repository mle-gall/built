# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""File dialogs start in the last folder the user chose, not in the folder
the program is installed in (Marco, 2026-09-07)."""
from __future__ import annotations

import os
from pathlib import Path


from core.settings import settings as preferences
from ingetrazo_render import shell_services as native

from ingetrazo_render.shell_files import ShellFileDialog, last_dir, remember, start_in


def _fresh():
    preferences().remove("files/last_dir")


def test_nothing_remembered_means_documents_or_a_fallback(tmp_path):
    _fresh()
    assert last_dir() and os.path.isdir(last_dir())
    assert last_dir(tmp_path) == str(tmp_path)            # the open document's folder
    assert start_in("", tmp_path) == str(tmp_path)
    assert start_in("lamina.pdf", tmp_path) == os.path.join(str(tmp_path), "lamina.pdf")


def test_the_chosen_folder_is_remembered_and_reused(tmp_path):
    _fresh()
    chosen = tmp_path / "planos" / "lamina.pdf"
    chosen.parent.mkdir()
    remember(chosen)
    assert last_dir() == str(chosen.parent)
    assert start_in("") == str(chosen.parent)
    assert start_in("vista.dxf") == str(chosen.parent / "vista.dxf")
    # a start directory the caller chose is respected
    other = tmp_path / "otra"; other.mkdir()
    assert start_in(str(other)) == str(other)
    assert start_in(str(other / "x.igz")) == str(other / "x.igz")
    # a remembered folder that vanished is not used
    preferences().setValue("files/last_dir", str(tmp_path / "borrada"))
    assert last_dir(tmp_path) == str(tmp_path)


def test_the_shim_feeds_the_dialog_and_learns_from_it(tmp_path, monkeypatch):
    _fresh()
    seen = {}

    def fake_save(title, name, filters, directory):
        seen["dir"] = str(Path(directory) / name)
        return str(tmp_path / "salida" / "lamina.pdf")
    (tmp_path / "salida").mkdir()
    monkeypatch.setattr(native, "save_file", fake_save)
    remember(tmp_path)
    path, _ = ShellFileDialog.getSaveFileName(None, "Export PDF…", "lamina.pdf", "PDF (*.pdf)")
    assert seen["dir"] == str(tmp_path / "lamina.pdf")   # started where the user was
    assert last_dir() == str(tmp_path / "salida")         # …and learnt the new folder

    def fake_open(title, filters, directory, multiple=False):
        seen["open"] = str(directory)
        return []
    monkeypatch.setattr(native, "open_files", fake_open)
    ShellFileDialog.getOpenFileName(None, "Open", "", "IGZ (*.igz)")
    assert seen["open"] == str(tmp_path / "salida")
    assert last_dir() == str(tmp_path / "salida")         # a cancel changes nothing


def test_with_nothing_remembered_a_dialog_starts_beside_the_open_document(tmp_path, monkeypatch):
    _fresh()
    (tmp_path / "proyecto").mkdir()

    class _Win:                      # the main window, as the shim sees it
        _current_path = tmp_path / "proyecto" / "poste.igz"

    class _Composer:                 # a composer: its parent is _window
        _window = _Win()
    seen = {}

    def fake_save(title, name, filters, directory):
        seen["dir"] = str(Path(directory) / name)
        return None
    monkeypatch.setattr(native, "save_file", fake_save)
    ShellFileDialog.getSaveFileName(_Composer(), "Export PDF…", "lamina.pdf", "PDF (*.pdf)")
    assert seen["dir"] == str(tmp_path / "proyecto" / "lamina.pdf")


def test_a_share_that_refuses_the_login_falls_back_instead_of_raising(tmp_path, monkeypatch):
    """#948: on Windows, Path.is_dir() on a network share that refuses the
    login raises (WinError 1326) instead of answering False, and every
    dialog died before showing."""
    _fresh()
    # A regular file in a directory prefix yields ENOTDIR, just as an
    # unavailable share makes the platform directory probe fail.
    inaccessible = tmp_path / "unavailable"
    inaccessible.write_text("")
    share = str(inaccessible / "projects")
    preferences().setValue("files/last_dir", share)
    assert last_dir(tmp_path) == str(tmp_path)
    assert start_in("lamina.pdf", tmp_path) == os.path.join(str(tmp_path), "lamina.pdf")
    assert start_in(share + "/plano.igz", tmp_path).endswith("plano.igz")
    remember(share + "/plano.igz")                      # nothing raised, nothing learnt
    assert preferences().value("files/last_dir") == share
    _fresh()
