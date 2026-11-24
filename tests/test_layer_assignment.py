# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Changing an object's layer where people look for it.

Rafael, 2026-09-16 (39:00–40:00): «no sé cómo cambiar el objeto de capa…
las propiedades del objeto… no lo veo. Botón derecho… no lo veo tampoco.
Asignar selección… algo no había seleccionado bien». The only road was
the Layers panel's button, silent when nothing was selected or no layer
highlighted. Now Entity Info has a Layer field, the right-click has a
Layer submenu, and all three go through one undoable command.
"""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtGui import QGuiApplication
from ingetrazo_core import Vec3 as QVector3D
from PySide6.QtWidgets import QApplication

_app = QApplication.instance() or QApplication([])

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from core.group import Group
from core.history import AssignLayerCommand, History
from core.layers import Layer, layer_of
from core.mesh import Mesh
from core.scene import Scene


def _square(mesh, x0=0.0):
    return mesh.add_face([QVector3D(x0, 0, 0), QVector3D(x0 + 1, 0, 0),
                          QVector3D(x0 + 1, 1, 0), QVector3D(x0, 1, 0)])


def test_assign_layer_command_moves_a_mixed_selection_and_undoes_exactly():
    scene = Scene()
    history = History(scene)
    scene.layers.append(Layer("Muros"))
    face = _square(scene.mesh)
    edge = scene.mesh.edges[0]
    g = Group(Mesh(), name="caja")
    g.layer = "Muros"
    scene.groups.append(g)

    history.execute(AssignLayerCommand([face, edge, g], "Mobiliario"))
    assert layer_of(face) == layer_of(edge) == layer_of(g) == "Mobiliario"
    assert scene.layer("Mobiliario") is not None      # created on the way

    history.undo()
    assert layer_of(face) == "Layer 0" and layer_of(edge) == "Layer 0"
    assert layer_of(g) == "Muros"
    assert scene.layer("Mobiliario") is None          # …and taken back
    history.redo()
    assert layer_of(g) == "Mobiliario"


def _window():
    from views.main_window import MainWindow
    win = MainWindow()
    win.show()
    _app.processEvents()
    return win


def test_inspector_tag_field_single_mixed_and_empty_selection(native_host):
    import json
    from tests.native_tree import action, tag_picker
    scene = native_host.viewport.scene
    scene.layers.append(Layer("Muros"))
    face = _square(scene.mesh)
    group = Group(Mesh(), name="caja")
    scene.groups.append(group)
    key, labels = tag_picker(native_host, [face])
    assert key == "Layer 0" and labels["Muros"] == "Muros"
    action(native_host, "tagpick", json.dumps("Muros"))
    assert layer_of(face) == "Muros" and native_host.viewport.history.undo()
    assert layer_of(face) == "Layer 0"
    group.layer = "Muros"
    key, labels = tag_picker(native_host, [face, group])
    assert not key and labels[key] == "(several)"
    action(native_host, "tagpick", json.dumps("Muros"))
    assert layer_of(face) == layer_of(group) == "Muros"
    assert tag_picker(native_host, []) is None


def test_right_click_layer_submenu_and_new_layer():
    from PySide6.QtWidgets import QMenu
    from views import chrome_forms
    win = _window()
    vp = win.viewport
    try:
        scene = vp.scene
        scene.layers.append(Layer("Muros"))
        g = Group(Mesh(), name="caja")
        scene.groups.append(g)
        scene.select([g])
        menu = QMenu()
        win._add_layer_submenu(menu, list(scene.selection))
        sub = [a for a in menu.actions() if a.menu() is not None][0].menu()
        names = [a.text() for a in sub.actions() if not a.isSeparator()]
        assert names[:2] == ["Layer 0", "Muros"] and names[2] == "New layer…"
        # …then what to do with the clicked object's own layer.
        assert names[3:] == ["Hide this layer", "Isolate this layer",
                             "Make this layer active"]
        ticked = [a.text() for a in sub.actions() if a.isChecked()]
        assert ticked == ["Layer 0"]
        [a for a in sub.actions() if a.text() == "Muros"][0].trigger()
        assert layer_of(g) == "Muros"

        win._assign_new_layer()
        form = chrome_forms.current(win)
        form.receive("text", "Jardín")
        form.press("ok")
        assert layer_of(g) == "Jardín" and scene.layer("Jardín") is not None
        vp.history.undo()
        assert layer_of(g) == "Muros" and scene.layer("Jardín") is None
    finally:
        win._saved_version = vp.scene.version
        win.close()


def test_drawing_assign_moves_the_selection_or_explains_what_is_missing(native_host, monkeypatch):
    from tests.native_tree import action
    vp = native_host.viewport
    g = Group(Mesh(), name="caja")
    vp.scene.groups.append(g)
    vp.scene.layers.append(Layer("Muros"))
    messages = []
    monkeypatch.setattr(vp, "flash_status", lambda message, *a: messages.append(message))
    action(native_host, "tagassign", "preview-tag:Muros")
    assert messages and "select" in messages[-1].lower() and not vp.history.undo_stack
    vp.scene.select([g])
    action(native_host, "tagassign", "preview-tag:missing")
    assert layer_of(g) == "Layer 0" and not vp.history.undo_stack
    action(native_host, "tagassign", "preview-tag:Muros")
    assert layer_of(g) == "Muros" and vp.history.undo()
    assert layer_of(g) == "Layer 0"
