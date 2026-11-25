# SPDX-License-Identifier: GPL-3.0-or-later
"""Project row materials use the Library and commit to the captured occurrences."""
import pytest

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from built_core.group import Group
from built_core.materials import Material
from tests.native_tree import action, group_rows
from built_render import chrome_sheet as sheets
from built_render import wgpu_library as library


@pytest.fixture
def rows(native_host):
    scene = native_host.viewport.scene
    a, b, c = [Group(name=n) for n in ("Wall", "Slab", "Desk")]
    scene.groups[:] = [a, b, c]
    scene.materials = {"Brick": Material("Brick", color=(.7, .3, .2)),
                       "Concrete": Material("Concrete", color=(.5, .5, .5))}
    scene.version += 1
    return a, b, c


def open_picker(host, row):
    assert "Choose material…" in group_rows(host, row)
    action(host, "paintrows", "i:" + row.uid)
    state = host.window._chrome_sheet
    assert state.provider.id == "tree-material"
    return rebuild(host)


def rebuild(host):
    state = host.window._chrome_sheet
    state.built = state.provider.build(sheets.Ctx(host.window, host.view))
    return state.built.wire()


def choose(host, *args):
    sheets.action(host.window, sheets.SEP.join(("tree-material", *args)))


def tiles(wire):
    return [item for block in wire["content"]["blocks"] if block["kind"] == "tiles"
            for item in block["items"]]


def test_choosing_material_keeps_the_selection_and_commits_once(native_host, rows):
    vp, (a, b, c) = native_host.viewport, rows
    vp.scene.select([a, b])
    wire = open_picker(native_host, a)
    assert vp.scene.selection == {a, b}
    assert [t["id"] for t in tiles(wire)] == ["m:Brick", "m:Concrete"]
    vp.scene.select([c])
    before = vp.history.revision
    choose(native_host, "open", "m:Brick")
    assert a.material == b.material == {"mat": "Brick", "color": [.7, .3, .2]}
    assert c.material is None and vp.history.revision == before + 1
    assert vp.history.undo() and a.material is None and b.material is None
    assert vp.history.redo()
    vp.scene.select([a,b])
    current = tiles(open_picker(native_host, a))
    assert [item["id"] for item in current if item.get("on")] == ["m:Brick"]
    choose(native_host, "verb", "clearpaint")
    assert a.material is None and b.material is None
    assert vp.history.undo() and a.material == b.material and a.material["mat"] == "Brick"


def test_search_categories_and_cancel(native_host, rows, monkeypatch):
    from unittest.mock import Mock
    renderer = Mock()
    monkeypatch.setattr(native_host.view, "renderer", renderer)
    a, b, _ = rows
    vp = native_host.viewport
    vp.scene.select([b])
    open_picker(native_host, a)
    choose(native_host, "search", "ral 9010")
    found = tiles(rebuild(native_host))
    assert len(found) == 1 and found[0]["id"] == "ral:RAL 9010"
    choose(native_host, "close")
    assert vp.scene.selection == {b} and a.material is None
    open_picker(native_host, a)
    choose(native_host, "side", "s:bundled")
    wire = rebuild(native_host)
    assert len([v for v in library._catalogue() if v.startswith("ral:")]) > 200
    assert any(item["id"].startswith("c:bundled") for item in wire["side"])
    choose(native_host, "search", "pool water")
    found = tiles(rebuild(native_host))
    assert any(t["id"] == "lib:water:1" and t["icon"] for t in found)
    assert renderer.ui_set_icons.called
    choose(native_host, "open", "lib:water:1")
    assert a.material["texture"]["sw"] == 3 and a.material["opacity"] == .62
    assert a.material["mat"] in vp.scene.materials and b.material is None


@pytest.mark.parametrize("name", ["Façade", ""])
def test_choosing_a_new_colour(native_host, rows, name):
    a = rows[0]
    open_picker(native_host, a)
    choose(native_host, "v", "tree_colour", "#0080ff")
    choose(native_host, "v", "tree_name", name)
    choose(native_host, "verb", "custompaint")
    assert a.material["color"] == pytest.approx([0., 128/255, 1.])
    assert a.material.get("mat", "") == name
    if name:
        assert name in native_host.viewport.scene.materials
    assert native_host.viewport.history.undo()
    assert a.material is None and name not in native_host.viewport.scene.materials


@pytest.mark.parametrize("change", ["deleted", "locked", "read-only"])
def test_late_refusal_is_atomic(native_host, rows, change):
    a, b, c = rows
    vp = native_host.viewport
    vp.scene.select([a,b])
    open_picker(native_host, a)
    if change == "deleted":
        vp.scene.groups.remove(b)
    elif change == "locked":
        b.locked = True
    else:
        vp.scene.worksession_read_only = True
    before = vp.history.revision
    choose(native_host, "open", "m:Brick")
    assert a.material is None and b.material is None
    assert vp.history.revision == before
