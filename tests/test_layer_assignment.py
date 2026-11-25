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

from built_core import Vec3 as QVector3D

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


def tag_form(host):
    from built_render import chrome_forms
    from built_render import context_menu_build, context_menu_run
    win = host.window
    menu = context_menu_build(win, host.view)
    assert menu.find("selectiontag").enabled
    assert context_menu_run(win, host.view, menu.kind, "selectiontag", menu.sel)
    return chrome_forms.current(win)


def test_right_click_tag_choice_and_new_tag(native_host):
    from built_render import chrome_forms
    scene = native_host.viewport.scene
    scene.layers.append(Layer("Muros"))
    group = Group(name="Box")
    scene.groups.append(group)
    scene.select([group])
    form = tag_form(native_host)
    assert form.fields[0].value == "Assign to Layer 0"
    form.receive("item", str(form.field("item").keys().index("Assign to Muros")))
    form.press("ok")
    assert layer_of(group) == "Muros"
    form = tag_form(native_host)
    form.receive("item", str(form.field("item").keys().index("New tag…")))
    form.press("ok")
    form = chrome_forms.current(native_host.window)
    form.receive("text", "Jardín")
    form.press("ok")
    assert layer_of(group) == "Jardín" and scene.layer("Jardín") is not None
    assert native_host.viewport.history.undo() and layer_of(group) == "Muros"
    assert scene.layer("Jardín") is None


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
