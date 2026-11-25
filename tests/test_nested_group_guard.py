# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Grouping with a group selected must refuse, not half-do it.

Nested groups are not supported yet. The actions filtered the selection down
to loose faces and edges, so selecting a group plus loose geometry and
grouping produced a group of the loose part with the group silently left out —
the wrong result dressed as success. Marco's flow is exactly that: group some
planks, place them in a bench, group again.

Make Group now ANSWERS instead of whispering into the status bar: it asks,
offering the two results that are actually reachable. Cancelling is the
refusal these tests are about, so they drive the dialog that way — see
tests/test_make_group_selection.py for the paths that accept.
"""
from __future__ import annotations


from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_core import Vec3 as QVector3D

import pytest  # noqa: E402

from core.history import AddFaceCommand, MakeGroupCommand  # noqa: E402


@pytest.fixture
def cancels(monkeypatch):
    """Answer any question the chrome asks with Cancel, and report whether
    one was actually asked — a silent refusal would pass these asserts too,
    and silence is the bug they exist to prevent."""
    from built_render import chrome_dialogs
    shown = {"count": 0, "titles": []}

    def ask(parent, title, text, buttons, *args, cancel=None, **kw):
        shown["count"] += 1
        shown["titles"].append(title)
        return cancel

    def standard(parent, tone, title, text, buttons, default):
        shown["count"] += 1
        shown["titles"].append(title)
        return 0

    monkeypatch.setattr(chrome_dialogs, "ask", ask)
    monkeypatch.setattr(chrome_dialogs, "_standard", standard)
    return shown


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


def _win_with_a_group_and_loose_geometry(native_host):
    win = native_host.window
    scene = win.viewport.scene
    hist = win.viewport.history
    hist.execute(AddFaceCommand([V(0, 0), V(1, 0), V(1, 1), V(0, 1)]))
    hist.execute(MakeGroupCommand(list(scene.mesh.faces),
                                  list(scene.mesh.edges)))
    hist.execute(AddFaceCommand([V(3, 0), V(4, 0), V(4, 1), V(3, 1)]))
    return win, scene


def test_grouping_a_group_with_loose_geometry_NESTS_it(native_host, cancels):
    """Esto se negaba, y con razón mientras entrar a un contenedor lo
    horneaba. Con la pila de contextos (2026-09-11) ya no: agrupar una cara
    suelta junto a un grupo mete la cara en la malla del contenedor nuevo y
    el grupo pasa a ser su hijo — lo habitual."""
    win, scene = _win_with_a_group_and_loose_geometry(native_host)
    antes = list(scene.groups)
    sueltas = len(scene.mesh.faces)
    scene.selection.clear()
    scene.selection.update(set(scene.mesh.faces) | set(scene.groups))
    win._on_make_group()
    assert cancels["count"] == 0, "ya no hay nada que preguntar"
    nuevos = [g for g in scene.groups if g not in antes]
    assert len(nuevos) == 1
    padre = nuevos[0]
    assert padre.children == antes, "el grupo viejo es ahora su hijo"
    assert len(padre.mesh.faces) == 1        # la cara suelta
    assert len(scene.mesh.faces) == sueltas - 1


def test_grouping_only_loose_geometry_still_works(native_host):
    win, scene = _win_with_a_group_and_loose_geometry(native_host)
    before = len(scene.groups)
    scene.selection.clear()
    scene.selection.update(scene.mesh.faces)
    win._on_make_group()
    assert len(scene.groups) == before + 1


def test_component_from_loose_geometry_plus_a_group_holds_both(native_host, monkeypatch):
    """It used to refuse; since issue #90 it makes ONE component holding the
    loose geometry and the group, still a group inside."""
    from built_render import chrome_forms
    win, scene = _win_with_a_group_and_loose_geometry(native_host)
    # the scale figure (a face-me billboard) never goes inside
    inner = [g for g in scene.groups
             if not getattr(g, "billboard", False)]
    before = len(scene.groups)
    scene.selection.clear()
    scene.selection.update(set(scene.mesh.faces) | set(scene.groups))
    win._on_make_component()
    form = chrome_forms.current(win)
    form.receive("text", "Banca")
    form.press("ok")
    assert len(scene.groups) == before - len(inner) + 1
    comp = [g for g in scene.groups if g not in inner
            and not getattr(g, "billboard", False)][0]
    assert comp.is_component() and comp.name == "Banca"
    assert comp.children == inner and not scene.mesh.faces
