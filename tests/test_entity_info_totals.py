# SPDX-License-Identifier: GPL-3.0-or-later
"""Entity Info adds up a multiple selection (#274).

With several faces or edges selected it only counted them; the usual totals
-- the faces' area, the edges' length -- are what a rough building model is
measured with."""
from __future__ import annotations

from ingetrazo_core import Vec3 as V
from tests.native_tree import inspector
from tests.shell_isolation import restore_shell_patches  # noqa: F401


def test_two_faces_and_three_edges_show_their_totals(native_host):
    scene = native_host.viewport.scene
    m = scene.mesh
    a = m.add_face([V(0, 0, 0), V(2, 0, 0), V(2, 3, 0), V(0, 3, 0)])  # 6 m²
    b = m.add_face([V(5, 0, 0), V(6, 0, 0), V(6, 1, 0), V(5, 1, 0)])  # 1 m²
    loose = [m.add_edge(V(0, 5, 0), V(4, 5, 0)),                      # 4 m
             m.add_edge(V(0, 6, 0), V(0.5, 6, 0))]                    # 0.5 m
    edge = m.find_edge(m.vertex_at(V(0, 0, 0)), m.vertex_at(V(0, 3, 0)))
    from core.units import fmt_area_with, fmt_len_with
    rows = inspector(native_host, [a, b])["Geometry"]
    assert rows["Area"] == fmt_area_with(7, scene.units)
    rows = inspector(native_host, loose + [edge])["Geometry"]
    assert rows["Length"] == fmt_len_with(7.5, scene.units)
    both = inspector(native_host, [a, b] + loose)["Geometry"]
    assert both == {"Faces": "2", "Edges": "2", "Area": fmt_area_with(7, scene.units),
                    "Length": fmt_len_with(4.5, scene.units)}


def test_a_face_with_an_opening_shows_its_net_area(native_host):
    """Entity Info reads the face as it is drawn: a 4×4 m face with a
    1×1 m opening is 15 m², alone and in a selection total."""
    m = native_host.viewport.scene.mesh
    f = m.add_face([V(0, 0, 0), V(4, 0, 0), V(4, 4, 0), V(0, 4, 0)],
                   [[V(1, 1, 0), V(1, 2, 0), V(2, 2, 0), V(2, 1, 0)]])
    g = m.add_face([V(6, 0, 0), V(7, 0, 0), V(7, 1, 0), V(6, 1, 0)])
    from core.units import fmt_area_with
    units = native_host.viewport.scene.units
    assert inspector(native_host, [f])["Geometry"]["Area"] == fmt_area_with(15, units)
    assert inspector(native_host, [f, g])["Geometry"]["Area"] == fmt_area_with(16, units)
