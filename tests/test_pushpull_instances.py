# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Empujar una cara que está DENTRO de un grupo o componente.

Dos reglas, y la segunda llegó después de usar la primera en obra:

1. Dentro del componente, el push llega a todas las copias — son la misma
   definición (Marco, 2026-09-04: «hice copias del componente, edito uno y
   los demás no cambian»).
2. **Desde fuera no se empuja.** La herramienta lo hacía —y en una instancia
   abría a tus espaldas una sesión de edición y compartía el resultado a
   todas las copias, cosa que se escribió como «mejor que lo habitual» el
   2026-06-10—. Dibujando de verdad eso se lee como que el modelo cambia
   donde no apuntaste (Marco, 2026-09-10: «cuando un dibujo esté agrupado y
   haga push sin entrar al grupo con doble clic no debería hacer push»). Es
   además la convención de los modeladores push/pull: *the Push/Pull tool cannot
   extrude objects that are a part of a Component or Group… double-click the
   Group or Component to edit it*.
"""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
import pytest
from ingetrazo_core import ScreenPoint, Input
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from core.group import Group, world_mesh
from core.mesh import Mesh
from ingetrazo_core.orient import is_closed, signed_volume
from ingetrazo_core.store_mesh import StoreMesh
from tools.base import ToolContext
from tools.pushpull import PushPullTool


def V(x, y, z=0.0):
    return QVector3D(x, y, z)


def _box(mesh, s=1.0):
    pts = [V(0, 0, 0), V(s, 0, 0), V(s, s, 0), V(0, s, 0),
           V(0, 0, s), V(s, 0, s), V(s, s, s), V(0, s, s)]
    quads = [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5),
             (2, 3, 7, 6), (3, 0, 4, 7)]
    faces = [mesh.add_face([pts[i] for i in q]) for q in quads]
    return faces[1]                                   # the top face


def _ctx(vp, mods=Input.NoModifier):
    return ToolContext(viewport=vp, world=V(0, 0), screen=ScreenPoint(0, 0),
                       modifiers=mods, snap=None)


def _instances(scene):
    proto = Mesh()
    top = _box(proto)
    a, b = Group(proto, name="Pilar"), Group(proto, name="Pilar")
    xb = QMatrix4x4()
    xb.translate(3.0, 0.0, 0.0)
    a.xform, b.xform = QMatrix4x4(), xb
    scene.groups += [a, b]
    scene.version += 1
    return proto, top, a, b


def _window(native_host):
    win = native_host.window
    return win, win.viewport


# ---- 2. desde fuera, no ----------------------------------------------------

def test_una_instancia_cerrada_no_se_empuja(native_host):
    win, vp = _window(native_host)
    scene = vp.scene
    proto, top, a, b = _instances(scene)
    pp = PushPullTool()
    vp.active_tool = pp
    pp.hovered_face = top
    pp._hover_group = b
    pp.on_click(_ctx(vp))
    assert not pp.dragging, "arrancó un push dentro de un grupo cerrado"
    assert scene.edit_group is None, "abrió una sesión a espaldas del usuario"
    assert b.mesh is proto and b.xform is not None    # sigue compartida
    assert abs(signed_volume(proto) - 1.0) < 1e-6     # y sin tocar
    assert not vp.history.undo_stack
    dicho = win.statusBar().currentMessage().lower()
    assert "group" in dicho and "double-click" in dicho, (
        "se negó sin decir por qué, que es lo que se lee como roto "
        f"(dijo: {dicho!r})")


def test_el_doble_clic_tampoco(native_host):
    """El doble clic repite la última distancia; sobre un grupo cerrado, no."""
    win, vp = _window(native_host)
    scene = vp.scene
    proto, top, a, b = _instances(scene)
    pp = PushPullTool()
    vp.active_tool = pp
    PushPullTool.last_distance = 1.5
    try:
        vp.pick_face_any = lambda x, y: (top, b)
        pp.on_double_click(_ctx(vp))
    finally:
        PushPullTool.last_distance = None
        del vp.pick_face_any
    assert not pp.dragging and scene.edit_group is None
    assert abs(signed_volume(proto) - 1.0) < 1e-6


def test_la_cara_de_un_grupo_cerrado_ni_se_sombrea(native_host):
    """Sombrearla sería prometer un empuje que no va a ocurrir."""
    win, vp = _window(native_host)
    scene = vp.scene
    _proto, top, _a, b = _instances(scene)
    pp = PushPullTool()
    vp.active_tool = pp
    vp.pick_face_any = lambda x, y: (top, b)
    try:
        pp.on_hover(_ctx(vp))
    finally:
        del vp.pick_face_any
    assert vp._hover_entity is None, "sombreó una cara que no se puede empujar"


# ---- 1. dentro, sí, y llega a todas las copias -----------------------------

@pytest.mark.parametrize("affine", [False, True], ids=["translated", "mirrored-and-scaled"])
def test_dentro_del_componente_el_push_llega_a_todas_las_copias(native_host, monkeypatch, affine):
    win, vp = _window(native_host)
    scene = vp.scene
    proto, _top, a, b = _instances(scene)
    placement = QMatrix4x4(b.xform)
    if affine:
        placement.rotate(90, 0, 0, 1)
        placement.scale(-2, 3, 0.5)
        b.xform = placement
    top_center = placement.map(V(0.5, 0.5, 1))
    ray = (top_center + V(0, 0, 5), V(0, 0, -1))
    monkeypatch.setattr(vp, "_pixel_to_ray", lambda x, y: ray)
    live_definition = isinstance(proto, StoreMesh)
    vp.begin_group_edit(b)
    assert scene.edit_group is b
    if live_definition:
        assert b.xform == placement and scene.mesh._instance_definition is proto
    else:
        assert b.xform is None and scene.mesh is b.mesh

    pp = PushPullTool()
    vp.active_tool = pp
    pp.on_hover(_ctx(vp))
    assert pp.hovered_face is not None
    assert (pp.hovered_face.centroid() - top_center).length() < 1e-6
    pp.on_click(_ctx(vp))
    assert pp.dragging
    monkeypatch.setattr(pp, "_infer_reference_distance", lambda ctx: 1.0)
    pp.on_hover(_ctx(vp))
    assert pp.on_value(vp, 2.0)
    assert vp.history.last_error is None and vp.history.last_rejected is None
    local_volume = 5.0 if affine else 3.0
    if live_definition:
        assert b.mesh is a.mesh is proto
        assert signed_volume(a.mesh) == pytest.approx(local_volume)
    vp.end_group_edit()

    assert a.mesh is b.mesh is proto and b.xform == placement
    assert is_closed(proto)
    assert signed_volume(world_mesh(a)) == pytest.approx(local_volume)
    wb = world_mesh(b)
    assert abs(signed_volume(wb)) == pytest.approx(15.0 if affine else 3.0)
    cap_height = max(v.position.z() for v in wb.vertices)
    assert cap_height == pytest.approx(top_center.z() + 2.0)
    cap = next(f for f in wb.faces if all(abs(v.z() - cap_height) < 1e-6 for v in f.vertices))
    assert (cap.centroid() - (top_center + V(0, 0, 2))).length() < 1e-6
    assert vp.history.undo()
    assert signed_volume(proto) == pytest.approx(1.0)
    assert vp.history.redo()
    assert signed_volume(proto) == pytest.approx(local_volume)
