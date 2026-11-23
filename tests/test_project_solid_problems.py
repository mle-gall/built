# SPDX-License-Identifier: GPL-3.0-or-later
"""Solid defects reached via Export and fixed in the Project inspector."""
import json

import pytest

from core.group import Group
from core.mesh import Mesh
from core.scene import Scene
from core.protocol.commands import run
from ingetrazo_core import Vec3, solid_inspector_sections, solid_problem_targets
from tests.solid_fixture import _box, _top
from tests.test_tree_links import _View, _Win
from views import wgpu_ui


@pytest.fixture
def project():
    scene = Scene()
    win, view = _Win(scene), _View()
    win._on_zoom_selection = lambda: None
    view._ui_lens = 2
    return scene, win, view


def inspect(project, key):
    scene, win, view = project
    wgpu_ui._tree(view, win)
    panel, payload = view.renderer.calls["ui_panel_set"]
    checks = json.loads(payload)["checks"]["*"]
    if key.startswith("i:"):
        entries = next(r[1] for code, r in checks.items() if code.startswith("solid/"))
        path = next(entry["path"] for entry in entries if entry["path"][-1] == key[2:])
        key = "exportitem:" + json.dumps(path)
    else:
        assert "active-solid" in checks
    wgpu_ui._tree_action(win, view, wgpu_ui.SEP.join(["item", key]))
    wgpu_ui._inspector(view, win)
    return view.renderer.calls["ui_set_inspector"]


def test_dialog_repairs_a_group_without_entering_it(project):
    scene, win, view = project
    _box(scene.mesh, skip_top=True)
    g = Group(Mesh(), name="Open box")
    _box(g.mesh, skip_top=True)
    scene.groups.append(g)
    title, sections = inspect(project, "i:" + g.uid)
    repair, = dict(sections)["Actions"]
    assert title == g.name and "hole" in repair[1]
    wgpu_ui._tree_action(win, view, "insp:" + repair[0])
    assert not scene.edit_group and scene.selection == {g}
    assert ("Watertight", "8.000 m³") in dict(solid_inspector_sections(scene, g))["Solid"]
    assert "4 open borders" in dict(solid_inspector_sections(scene, None))["Solid"][0][1]
    assert win.viewport.history.undo()
    assert "4 open borders" in dict(solid_inspector_sections(scene, g))["Solid"][0][1]


def test_status_says_reversed_on_a_closed_wrong_box(project):
    scene, _, _ = project
    g = Group(Mesh(), name="Wrong winding")
    _box(g.mesh)
    _top(g.mesh).loop.reverse()
    scene.groups.append(g)
    _, sections = inspect(project, "i:" + g.uid)
    assert dict(sections)["Solid"] == [("Faces reversed", "1 reversed faces")]
    assert all("m³" not in v for k, rows in sections for _, v in rows)


def test_instances_of_one_component_are_listed_once(project):
    scene, _, _ = project
    shared = Mesh()
    _box(shared, skip_top=True)
    a, b = Group(shared, name="Chair"), Group(shared, name="Chair copy")
    scene.groups += [a, b]
    broken, _, _ = solid_problem_targets(scene)
    assert broken == [a]


def test_unrepairable_problem_is_shown_with_its_reason(project):
    scene, win, _ = project
    _box(scene.mesh)
    _box(scene.mesh, x0=2.0, z0=2.0)
    title, sections = inspect(project, "solid:active")
    assert title == "Loose geometry" and "Actions" not in dict(sections)
    assert "separate the solids" in dict(sections)["Manual repairs"][0][1]
    assert not run(win.viewport.history, "repair_solid", placement=None, fault="gone")["ok"]
    assert not win.viewport.history.undo_stack


def test_repair_only_the_chosen_loose_fault_and_reject_stale_keys(project):
    scene, win, view = project
    _box(scene.mesh, skip_top=True)
    scene.mesh.add_edge(Vec3(8, 0, 0), Vec3(9, 0, 0))
    _, sections = inspect(project, "solid:active")
    hole, stray = dict(sections)["Actions"]
    wgpu_ui._tree_action(win, view, "insp:" + hole[0])
    wgpu_ui._tree_action(win, view, "insp:" + hole[0])
    assert len(win.viewport.history.undo_stack) == 1
    assert dict(solid_inspector_sections(scene, None))["Solid"] == [("Not watertight", "1 stray edges")]
    wgpu_ui._tree_action(win, view, "insp:" + stray[0])
    assert len(win.viewport.history.undo_stack) == 2
    assert "Watertight" in dict(dict(solid_inspector_sections(scene, None))["Solid"])
    assert win.viewport.history.undo()
    assert "stray" in dict(solid_inspector_sections(scene, None))["Solid"][0][1]


def test_locked_target_rejects_a_fault_from_before_the_lock(project):
    scene, win, _ = project
    broken = Group(Mesh(), name="Stool leg")
    _box(broken.mesh, skip_top=True)
    parent = Group(name="Stool")
    parent.adopt([broken])
    scene.groups.append(parent)
    assert solid_problem_targets(scene)[0] == [broken]
    _, sections = inspect(project, "i:" + broken.uid)
    action, = dict(sections)["Actions"]
    parent.locked = True
    key = action[0].removeprefix("solidfault:")
    assert not run(win.viewport.history, "repair_solid", placement=scene.occurrence_path(broken), fault=key)["ok"]
    assert "Actions" not in dict(solid_inspector_sections(scene, broken))


def test_scoped_status_fix_updates_readiness_and_undo(project, tmp_path):
    from core.deliveries import create_delivery
    scene, win, view = project
    scene.plugin_data["bim_registry"] = {"structure": {"nodes": {
        "walls": {"name": "Walls", "parent": None, "recipe": {"class": "IfcWall"}}
    }}}
    walls = [Group(name="North"), Group(name="South")]
    for g in walls:
        g.ifc = {"class": "IfcWall"}
        g.ext = {"structure": {"node": "walls"}}
    scene.groups.extend([*walls, Group(name="Outside")])
    history = win.viewport.history
    create_delivery(history, dict(name="Walls IFC", format="ifc", scope="node", node="walls",
                                  destination=str(tmp_path / "walls.ifc")))

    def report():
        wgpu_ui._tree(view, win)
        return json.loads(view.renderer.calls["ui_panel_set"][1])

    initial = report()
    assert (initial["ready"], initial["total"]) == (0, 2)
    entries = initial["checks"]["n:walls"]["status/Pset_WallCommon"][1]
    paths = [item["path"] for item in entries]
    target = scene.resolve_occurrence(paths[0])
    wgpu_ui._tree_action(win, view, "eye" + wgpu_ui.SEP + "exportitem:" + json.dumps(paths[0]))
    assert target.hidden and history.undo() and not target.hidden
    before = len(history.undo_stack)
    args = dict(placements=paths, path=["psets", entries[0]["fix"], "Status"],
                value="EXISTING", property_kind="label")
    wgpu_ui._tree_action(win, view, "exportstatus" + wgpu_ui.SEP + json.dumps(args))
    assert len(history.undo_stack) == before + 1 and report()["ready"] == 2
    assert history.undo() and report()["ready"] == 0
