# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""What a sheet frame shows (ingetrazo_core sheets::visibility) is what the
composer showed by applying the frame's saved view to the live scene and
reading the rows (SavedView.apply_state + views/wgpu_scene's placement
walk), without touching the scene: tags, tags that do not print, nested tag folders, Hide on
groups and nested placements, the view's hidden objects and folders."""
from __future__ import annotations

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest  # noqa: E402

import ingetrazo_core as NATIVE  # noqa: E402


def _scene():
    from ingetrazo_core import Vec3 as V
    from core.group import Group
    from core.layers import Layer
    from core.mesh import Mesh
    from core.saved_views import SavedView
    from core.scene import Scene
    s = Scene()
    for name, folder in (("Muros", "Arq"), ("Losa", "Arq"), ("Mobiliario", "Int"), ("Ejes", None),
                         ("Rejas", "Int"), ("Notas", None)):
        s.layers.append(Layer(name, folder=folder, printable=name != "Notas"))
    s.layer_folders = ["Arq", "Int", "Det"]
    s.folder_parents = {"Int": "Arq"}
    s.folder_visibility = {"Det": False}

    def box(x):
        m = Mesh()
        p = [V(x + a, b, c) for c in (0, 1) for b in (0, 1) for a in (0, 1)]
        for loop in ((0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4)):
            m.add_face([p[i] for i in loop])
        return m
    groups = []
    for i, tag in enumerate(("Muros", "Losa", "Mobiliario", "Ejes", "Rejas", None, "Notas")):
        g = Group(box(3 * i), f"g{i}")
        g.layer = tag
        groups.append(g)
    nest = Group(box(30), "nest")
    nest.layer = "Muros"
    inner = Group(box(33), "inner")
    inner.layer = "Mobiliario"
    deep = Group(box(36), "deep")
    deep.layer = "Ejes"
    inner.children.append(deep)
    nest.children.append(inner)
    groups[2].hidden = True
    deep.hidden = False
    s.groups.extend(groups + [nest])
    s.saved_views.append(SavedView("all"))
    s.saved_views.append(SavedView("no walls", hidden_layers=["Muros"]))
    s.saved_views.append(SavedView("folders", hidden_folders=["Int"]))
    s.saved_views.append(SavedView("objects", hidden_objects=[groups[0].uid, inner.uid]))
    s.saved_views.append(SavedView("mixed", hidden_layers=["Ejes"], hidden_folders=["Arq"],
                                   hidden_objects=[]))
    return s


def _reference(scene, view):
    """The composer's way: apply the view, leave out what does not print,
    walk the placements."""
    from types import SimpleNamespace
    from core.layers import hide_unprinted
    from views.wgpu_scene import _placement_records, _placement_state
    if view is not None:
        view.apply_state(scene)
    hide_unprinted(scene)
    vp = SimpleNamespace(scene=scene)
    state = _placement_state(scene)
    return {id(g): bool(flags & 1) for g, _, _, flags, _c in _placement_records(scene, scene.groups, vp, state)}


@pytest.mark.parametrize("view", [None, "all", "no walls", "folders", "objects", "mixed"])
def test_the_frame_hides_what_the_view_hides_without_touching_the_scene(view):
    scene = _scene()
    handles = NATIVE.doc_tree(list(scene.groups))[0]
    v = next((x for x in scene.saved_views if x.name == view), None)
    before = ([ly.visible for ly in scene.layers], dict(scene.folder_visibility),
              [g.hidden for g in handles])
    got = list(NATIVE.sheet_frame_hidden(scene._root, list(scene.groups), list(handles) + [None], v))
    assert ([ly.visible for ly in scene.layers], dict(scene.folder_visibility),
            [g.hidden for g in handles]) == before                 # the live scene untouched
    want = _reference(_scene_like(scene), _view_like(scene, view))
    want = [int(want.get(id(h), True)) for h in NATIVE.doc_tree(list(_LAST.groups))[0]] + [0]
    assert got == want


_LAST = None


def _scene_like(scene):
    """A second, identical scene for the reference to change."""
    global _LAST
    _LAST = _scene()
    return _LAST


def _view_like(scene, name):
    return next((x for x in _LAST.saved_views if x.name == name), None)
