# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The native rail, command registry and STL import defaults."""
from threading import get_ident

from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_save_writes_the_model_from_the_file_menu(native_host, monkeypatch, tmp_path):
    from views.filedialogs import file_dialogs
    path = tmp_path / "saved.built"
    monkeypatch.setattr(file_dialogs, "getSaveFileName", lambda *_: (str(path), ""))
    native_host.menu("save")
    assert path.is_file() and native_host.window._current_path == path


def test_simplify_mesh_is_only_in_the_edit_menu():
    from ingetrazo_render import command_registry, rail_layout
    assert any(row[1] == "Edit" and row[5] == "verb:simplify_mesh"
               for row in command_registry([], []))
    assert all(key != "simplify_mesh" for group in rail_layout() for key, _ in group)


def test_stl_simplification_defaults_to_principal_planes(native_host, monkeypatch, tmp_path):
    from core.settings import settings
    from tests.test_import_failure_reported import _GOOD_STL
    from views.filedialogs import file_dialogs
    settings().setValue("import/stl_simplify", "true")
    path = tmp_path / "sample.stl"
    path.write_bytes(_GOOD_STL)
    monkeypatch.setattr(file_dialogs, "getOpenFileName", lambda *args: (str(path), ""))
    forms = []
    monkeypatch.setattr("ingetrazo_render.chrome_forms.show", lambda _w, form: forms.append(form))
    native_host.menu("import:stl")
    assert forms[0].values()["simplify"] == "principal"
    forms[0].press("cancel")
    assert not native_host.viewport.history.undo_stack


def test_stl_parse_reports_worker_progress_on_the_calling_thread(native_host, tmp_path):
    from tests.test_import_failure_reported import _GOOD_STL
    path = tmp_path / "triangle.stl"
    path.write_bytes(_GOOD_STL)
    threads = []
    mesh, error = native_host.window._parse_stl_threaded(
        path, 1., "none", lambda *_: threads.append(get_ident()))
    assert error is None and len(mesh.faces) == 1
    assert set(threads) == {get_ident()}
