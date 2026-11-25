# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Right-click on loose geometry that sits flush on a group keeps it selected.

#744 (@duaomarfs): a block modelled on top of a group, selected, then
right-clicked — the group took the click, the selection was lost and
«Make Group» with it. #378 covered a loose face clearly in front of the
group; where the two are coplanar (a face drawn on the group's surface, the
foot of the block, its bottom edges) the group still won."""
from __future__ import annotations


from tests.test_pick_loose_in_front_of_group import (  # noqa: E402
    V, _floor_group, _loose_lid, _vp)


def _flush_lid():
    vp = _vp()
    g = _floor_group(vp)
    lid = _loose_lid(vp, z=0.0)
    edges = [e for e in vp.scene.mesh.edges if lid in e.faces]
    return vp, g, lid, edges


def test_right_click_on_a_selected_face_flush_on_a_group():
    vp, _, lid, edges = _flush_lid()
    vp.scene.select([lid, *edges])
    vp._context_pick(*vp._world_to_pixel(V(3, 3, 0)))
    assert set(vp.scene.selection) == {lid, *edges}


def test_right_click_on_a_selected_edge_flush_on_a_group():
    vp, _, lid, edges = _flush_lid()
    vp.scene.select([lid, *edges])
    vp._context_pick(*vp._world_to_pixel(V(3, 2, 0)))
    assert set(vp.scene.selection) == {lid, *edges}


def test_right_click_on_the_group_beside_it_still_picks_the_group():
    vp, g, lid, edges = _flush_lid()
    vp.scene.select([lid, *edges])
    vp._context_pick(*vp._world_to_pixel(V(5, 5, 0)))
    assert list(vp.scene.selection) == [g]


def test_right_click_rect_label_precedes_group_without_probing_later_hits():
    from unittest.mock import Mock
    from views.document_interaction import DocumentInteraction
    label = object()
    vp = Mock()
    vp.scene.selection = set()
    vp.pick_edge.return_value = vp.pick_face.return_value = None
    vp.pick_text_label.return_value = label
    assert DocumentInteraction._context_pick(vp, 2., 3.) is label
    vp.pick_text_label.assert_called_once_with(2., 3., rect_only=True)
    vp.pick_group.assert_not_called()
    vp.scene.select.assert_called_once_with([label])
    vp.update.assert_called_once_with()
