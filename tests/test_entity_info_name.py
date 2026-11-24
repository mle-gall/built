# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Issue #214: a group or component is named in Entity Info, where
everyone looks for it — one undo step, per object."""
from __future__ import annotations

from ingetrazo_core import Vec3 as QVector3D

from core.group import Group
from core.mesh import Mesh
from tests.native_tree import action, inspector
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from views.wgpu_ui import _edit


def V(x, y, z=0.0):
    return QVector3D(float(x), float(y), float(z))


def _group(scene, name, x=0.0):
    m = Mesh()
    m.add_face([V(x, 0), V(x + 1, 0), V(x + 1, 1), V(x, 1)])
    g = Group(m, name=name)
    scene.groups.append(g)
    return g


def test_the_field_shows_only_for_one_group(native_host):
    scene = native_host.viewport.scene
    a, b = _group(scene, "Cámara"), _group(scene, "Brida", 3)
    for group in (a, b):
        assert inspector(native_host, [group])["Item"]["Name"] == _edit(
            f"groupname:{group.uid}", group.name)
    assert "Item" not in inspector(native_host, [a, b])


def test_renaming_is_one_undo_step(native_host):
    vp = native_host.viewport
    g = _group(vp.scene, "Group")
    vp.scene.select([g])
    revision = vp.history.revision
    action(native_host, "edit", f"groupname:{g.uid}", "Cámara de vacío")
    assert g.name == "Cámara de vacío" and vp.history.revision == revision + 1
    assert vp.history.undo() and g.name == "Group"


def test_an_empty_name_is_not_taken(native_host):
    vp = native_host.viewport
    g = _group(vp.scene, "Tapa")
    vp.scene.select([g])
    revision = vp.history.revision
    action(native_host, "edit", f"groupname:{g.uid}", "   ")
    assert vp.history.revision == revision
    assert inspector(native_host, [g])["Item"]["Name"] == _edit(f"groupname:{g.uid}", "Tapa")
