# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Folder and tag headings in the native Project tree and Drawing lens."""
import ingetrazo_core
import time

from core import layers as L
from core.group import Group
from core.layers import DEFAULT_LAYER, Layer
from tests.native_tree import action
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_browser_layer_order import _group, _container
from tests.test_project_drawing_filters import Project


def test_only_matching_folders_and_their_ancestors_get_a_heading():
    p = _drawing()
    p.scene.layer_folders.append("Empty")
    p.scene.version += 1
    rows = {k: v for k, v in p.action("filter", "roof").items() if k.startswith("preview-")}
    assert list(rows) == ["preview-folder:Building", "preview-folder:Envelope", "preview-tag:Roof"]
    assert [state[0] for _, state in rows.values()] == [0, 1, 2]


def test_the_filter_leaves_no_empty_heading():
    from core import structure
    from tests.test_project_drawing_filters import Project
    from tests.test_structure import model
    scene, registry, house, shed = model()
    project = Project(scene=scene)
    history = project.win.viewport.history
    for group, node in [(house, "sections"), (shed, "ref")]:
        history.execute(structure.file(scene, [group.uid], node))
    # Only the matching placement's filing branch and its ancestors remain.
    assert "n:ref" in project.action("filter", "Survey point")
    assert "n:sections" not in project.rows()
    house.hidden = True
    scene.version += 1
    rows = project.action("filter", "is:hidden")
    assert "n:sections" in rows and "n:ref" not in rows
    project.action("filter", "")
    project.action("expandall", "")
    assert {"n:sections", "n:ref"} <= project.rows().keys()


def _drawing():
    p = Project(Layer("Walls", folder="Building"), Layer("Roof", folder="Envelope"), Layer("Site"))
    p.scene.layer_folders = ["Building", "Envelope"]
    p.scene.folder_parents = {"Envelope": "Building"}
    p.scene.groups[:] = [_group("Shed", "Walls"), _group("House", "Roof"), _group("Site", "Site")]
    p.scene.version += 1
    p.action("lens", "1")
    return p


def test_folder_disclosure_survives_document_edits_and_is_local_to_the_view():
    p = _drawing()
    original = list(p.scene.groups)
    rows = p.action("toggle", "preview-folder:Building")
    assert rows["preview-tag:Walls"][1][0] == rows["preview-folder:Envelope"][1][0] == 1
    assert "preview-tag:Roof" not in rows
    rows = p.action("toggle", "preview-folder:Envelope")
    assert rows["preview-tag:Roof"][1][0] == 2
    p.action("eye", "preview-tag:Roof")
    assert "preview-tag:Roof" in p.rows()
    p.action("toggle", "preview-folder:Building")
    assert "preview-folder:Envelope" not in p.rows()
    assert "preview-tag:Roof" in p.action("toggle", "preview-folder:Building")
    assert "preview-tag:Walls" not in _drawing().rows()
    assert list(p.scene.groups) == original


def test_folder_filter_keeps_matching_ancestors_and_restores_disclosure():
    p = _drawing()
    before = list(p.rows())
    rows = p.action("filter", "roof")
    assert list(k for k in rows if k.startswith("preview-")) == [
        "preview-folder:Building", "preview-folder:Envelope", "preview-tag:Roof"]
    assert list(p.action("filter", "")) == before


def test_expand_all_and_collapse_all_reach_filing_headings():
    from tests.test_structure import model
    p = Project(scene=model()[0])
    p.rows()
    assert {"n:ride", "n:track", "n:sections", "n:ref"} <= p.action("expandall", "").keys()
    rows = p.action("collapseall", "")
    assert "n:ride" in rows and "n:track" not in rows
    p = _drawing()
    assert p.action("expandall", "")["preview-tag:Roof"][1][0] == 2
    assert "preview-tag:Roof" not in p.action("collapseall", "")


def test_sort_and_refresh_keep_heading_keys_and_group_ownership():
    p = _drawing()
    outer, inner = "preview-folder:Building", "preview-folder:Envelope"
    p.action("toggle", outer)
    p.action("toggle", inner)
    original = list(p.scene.groups)
    for sort in ("1", "0"):
        p.action("sort", sort)
        rows = p.rows()
        assert rows[inner][1][0] == rows[outer][1][0] + 1
        assert rows["preview-tag:Roof"][1][0] == rows[inner][1][0] + 1
    p.scene.layers.append(Layer("Doors", folder="Envelope"))
    p.scene.version += 1
    assert p.rows()["preview-tag:Doors"][1][0] == 2
    assert list(p.scene.groups) == original


def test_counts_exclude_nested_parts_and_follow_retagging():
    p = _drawing()
    kid = _group("Window", "Roof")
    room = _container("Room", [kid], layer="Walls")
    p.scene.groups.append(room)
    p.scene.version += 1
    p.action("toggle", "preview-folder:Building")
    rows = p.action("toggle", "preview-folder:Envelope")
    assert [rows[k][1][1] for k in ["preview-folder:Building", "preview-tag:Walls", "preview-tag:Roof"]] == [3, 2, 1]
    from core.protocol.commands import set_tag
    assert set_tag(p.win.viewport.history, [room.uid], "Roof")["ok"]
    rows = p.rows()
    assert [rows[k][1][1] for k in ["preview-tag:Walls", "preview-tag:Roof"]] == [1, 2]


def test_drawing_eye_lock_and_folder_eye_leave_objects_intact():
    p = _drawing()
    s = p.scene
    p.action("toggle", "preview-folder:Building")
    for verb, field, before in [("eye", "visible", True), ("lock", "locked", False)]:
        p.action(verb, "preview-tag:Walls")
        assert getattr(s.layer("Walls"), field) != before
        assert p.win.viewport.history.undo() and getattr(s.layer("Walls"), field) == before
    p.action("eye", "preview-folder:Building")
    assert all(ly.visible and not ingetrazo_core.tag_edits.effective_visible(s, ly) for ly in [s.layer("Walls"), s.layer("Roof")])
    assert ingetrazo_core.tag_edits.effective_visible(s, s.layer("Site"))
    assert all(not g.hidden and not g.locked for g in s.groups)


def test_select_items_by_tag_leaves_an_open_group_and_respects_locks(native_host):
    vp = native_host.viewport
    s = vp.scene
    kid = _group("Window", "Walls")
    host = _container("House", [kid])
    a, b = _group("A", "Walls"), _group("B", "Walls")
    s.groups[:] = [a, b, host]
    s.layers.append(Layer("Walls"))
    s.version += 1
    vp.begin_group_edit(host)
    assert s.edit_group is host
    action(native_host, "tagselect", "preview-tag:Walls")
    assert s.edit_group is None and set(s.selection) == {a, b}
    action(native_host, "lock", "preview-tag:Walls")
    action(native_host, "tagselect", "preview-tag:Walls")
    assert not s.selection and s.layer("Walls").locked


def test_ten_thousand_placements_counted_under_their_tag_once():
    p = Project(Layer("Walls"), Layer("Roof"))
    groups = [Group(name=f"G{i}") for i in range(10_000)]
    for i, group in enumerate(groups):
        group.layer = ("Walls", "Roof", None)[i % 3]
    p.scene.groups = groups
    start = time.perf_counter()
    rows = p.rows()
    assert [rows["preview-tag:" + tag][1][1] for tag in ("Walls", "Roof", DEFAULT_LAYER)] == [3334, 3333, 3333]
    assert time.perf_counter() - start < 10
