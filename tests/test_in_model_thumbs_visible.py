# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""The «In model» list draws the pictures of the rows on screen only: a
plant of 1 815 components drew every one at each refresh, a quarter of
opening it (#158). Scrolled to, a row gets its picture; the picture is
the one drawn before."""
from __future__ import annotations
from tests.test_library_sheet import win  # noqa: F401
from tests.shell_isolation import restore_shell_patches  # noqa: F401

import pytest

from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V

from core.group import Group
from core.mesh import Mesh
from ingetrazo_render import component_thumbnail, component_signature


def _component(i):
    m = Mesh()
    s = 1.0 + i * 0.01
    m.add_face([V(0, 0, 0), V(s, 0, 0), V(s, s, 0), V(0, s, 0)])
    g = Group(m, name=f"C{i:03d}")
    xf = QMatrix4x4()
    xf.translate(i * 2.0, 0.0, 0.0)
    g.xform = xf
    return g


def test_only_rows_on_screen_are_drawn(win, monkeypatch):
    from tests.test_replace_component import _component_sheet
    from tests.test_library_sheet import act
    drawn = []
    from core import lazy_mesh
    plan = lazy_mesh.plan_of
    monkeypatch.setattr(lazy_mesh, "plan_of", lambda mesh: (drawn.append(id(mesh)), plan(mesh))[1])
    scene = win.viewport.scene
    scene.groups.extend(_component(i) for i in range(120))
    scene.version += 1
    tiles = _component_sheet(win)
    assert len(tiles) == 120 and drawn == []
    for visible in (tiles[:8], tiles[-4:]):
        act(win, "component-previews", *(t["icon"] for t in visible))
        by_name = {g.name: id(g.mesh) for g in scene.groups}
        assert drawn[-len(visible):] == [by_name[t["label"]] for t in visible]
    assert len(drawn) == 12


def test_nested_lazy_thumbnail_matches_eager_without_materialization(monkeypatch, tmp_path):
    import pytest
    from core.scene import Scene
    from core.mesh import DeferredMesh
    from formats import igz
    from tests.test_deferred_mesh import _open
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 1)
    proto = _component(0).mesh
    proto.faces[0].attrs["color"] = [0.8, 0.2, 0.4]
    children = []
    for i in range(13):
        g = Group(proto, name=str(i))
        g.xform = QMatrix4x4()
        g.xform.translate(i % 4, i // 4, i * 0.1)
        g.xform.rotate(i * 7, 0, 0, 1)
        children.append(g)
    root = Group(name="assembly")
    root.adopt(children)
    sc = Scene()
    sc.groups.append(root)
    path = tmp_path / "thumb.igz"
    igz.save_scene(sc, path)
    eager = _open(monkeypatch, path, False).groups[0]
    lazy = _open(monkeypatch, path, True).groups[0]
    assert type(lazy.children[0].mesh) is DeferredMesh
    expected = component_thumbnail(eager, 80, max_faces=5).rgba()
    monkeypatch.setattr(DeferredMesh, "_materialize", lambda self: pytest.fail(
        "a thumbnail built editable mesh topology"))
    assert lazy.children[0].mesh is lazy.children[-1].mesh
    assert [row[1:] for row in component_signature(lazy)] == [
        row[1:] for row in component_signature(eager)]
    actual = component_thumbnail(lazy, 80, max_faces=5).rgba()
    assert actual == expected
    # Signature validation and a second picture must remain lazy too.
    assert component_thumbnail(lazy, 80, max_faces=5).rgba() == expected
