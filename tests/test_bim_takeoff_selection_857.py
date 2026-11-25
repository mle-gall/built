# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Selecting does not re-measure the BIM takeoff (#857, @buildsmart888).

The BIM dock re-measured every tagged object — a world mesh and a volume
each — on every scene change, and a selection is one: on a rebar model of
238 tagged bars a box selection waited 25 s. The takeoff reads geometry
and tags, never the selection, so a view-only change keeps it; an edit
or an undo still re-measures."""
from __future__ import annotations

import pytest
from built_core import Matrix4 as QMatrix4x4, Vec3 as V
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from tests.test_bim_takeoff_untagged_moves import measure
from tests.native_tree import inspector

from built_core.group import Group  # noqa: E402
from built_core.mesh import Mesh  # noqa: E402


def _bar(x):
    m = Mesh()
    p = [V(0, 0, 0), V(1, 0, 0), V(1, 1, 0), V(0, 1, 0),
         V(0, 0, 1), V(1, 0, 1), V(1, 1, 1), V(0, 1, 1)]
    for q in ([0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5],
              [2, 3, 7, 6], [3, 0, 4, 7]):
        m.add_face([p[i] for i in q])
    g = Group(m, f"bar {x}")
    g.xform = QMatrix4x4()
    g.xform.translate(x * 2.0, 0, 0)
    g.ifc = {"class": "IfcReinforcingBar", "name": f"bar {x}", "id": x + 1}
    return g


def test_a_selection_keeps_the_takeoff_and_an_edit_remeasures(native_host, monkeypatch):
    from core import bim
    from tests.test_bim_takeoff_untagged_moves import _counting
    scene = native_host.viewport.scene
    bars = [_bar(i) for i in range(3)]
    scene.groups += bars
    scene.version += 1
    assert measure(native_host).count == 3
    calls = _counting(monkeypatch)
    for selection in ([bars[0]], bars, []):
        inspector(native_host, selection)
    assert calls == []
    bars[0].xform.scale(2.)
    scene.version += 1
    rows = inspector(native_host, [bars[0]])
    assert calls == [1]
    assert float(rows["Quantities"]["kg"]) == pytest.approx(8 * bim.STEEL_DENSITY)


def test_an_edit_remeasures_only_the_object_it_touched():
    from core.bim import collect_objects
    from built_core.scene import Scene
    scene = Scene()
    bars = [_bar(i) for i in range(3)]
    scene.groups += bars
    memo = {}
    first = collect_objects(scene, memo)
    again = collect_objects(scene, memo)
    assert all(a is b for a, b in zip(first, again))      # nothing re-measured
    bars[1].xform.scale(2.0)
    third = collect_objects(scene, memo)
    assert third[0] is first[0] and third[2] is first[2]
    assert third[1] is not first[1]
    assert third[1]["volume"] == pytest.approx(8.0)
    bars[2].ifc = dict(bars[2].ifc, name="renamed")
    fourth = collect_objects(scene, memo)
    assert fourth[2] is not third[2] and fourth[2]["name"] == "renamed"
    # Untagged or removed objects leave the memo.
    scene.groups.remove(bars[0])
    collect_objects(scene, memo)
    assert len(memo) == 2
    # Without a memo every call measures afresh, as the IFC export wants.
    assert collect_objects(scene)[0] is not collect_objects(scene)[0]


def test_takeoff_waits_until_asked_and_reports_elapsed(native_host, monkeypatch):
    from tests.test_bim_takeoff_untagged_moves import _counting
    vp = native_host.viewport
    vp.scene.groups.append(_bar(0))
    vp.scene.version += 1
    calls, status = _counting(monkeypatch), []
    monkeypatch.setattr(vp, "flash_status", status.append)
    assert measure(native_host, auto=True) is None and not calls
    assert measure(native_host).count == 1 and calls == [1]
    assert status[-1].startswith("Measured 1 items in ") and status[-1].endswith(" s")
