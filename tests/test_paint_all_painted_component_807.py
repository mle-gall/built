# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Painting a component whose faces all carry their own material (#807).

The .skp of the report has cabinet parts as components with every face
painted: painting one as a whole keeps those materials (see #408), so the click changed nothing on screen and read as «it won't
let me paint components». The status bar now says why and what to do."""
from __future__ import annotations


from built_core import Matrix4 as QMatrix4x4, Vec3 as V


from built_core.group import Group  # noqa: E402
from built_core.mesh import Mesh  # noqa: E402
from tools.paint import PaintTool  # noqa: E402


def _square(paint):
    m = Mesh()
    f = m.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    if paint:
        f.attrs["color"] = [0.2, 0.4, 0.6]
    return m


def test_a_component_with_every_face_painted_is_told_apart():
    assert PaintTool._every_face_painted(Group(_square(True), "panel"))
    assert not PaintTool._every_face_painted(Group(_square(False), "panel"))


def test_a_nested_unpainted_part_still_takes_the_paint():
    outer = Group(_square(True), "cabinet")
    inner = Group(_square(False), "door")
    inner.xform = QMatrix4x4()
    outer.adopt([inner])
    assert not PaintTool._every_face_painted(outer)


def test_the_click_says_why_nothing_changed():
    from types import SimpleNamespace as NS
    flashed = []
    group = Group(_square(True), "panela_porta")
    vp = NS(flash_status=flashed.append, history=NS(execute=lambda c: None),
            update=lambda: None, pick_group=lambda x, y: group,
            pick_face_any=lambda x, y: (group.mesh.faces[0], group))
    tool = PaintTool()
    tool._remember_current = lambda vp: None
    obj = tool._container_under(vp, group, 0, 0)
    assert obj is group
    name = "panela_porta"
    msg = ("{name}: every face has its own material, so nothing changed "
           "— open it and paint the faces")
    from built_core.i18n import tr
    assert tr(msg, name=name).startswith(name)
