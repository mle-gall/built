# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Assembly geometry and the cut list report."""
from __future__ import annotations

from tests.shell_isolation import restore_shell_patches  # noqa: F401
import pytest
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as QVector3D

from core.group import Group
from core.mesh import Mesh
from core.parts import (cut_list, cut_list_text, part_material,
                        part_points, part_rows, part_size)


_QUADS = ((0, 2, 3, 1), (4, 5, 7, 6), (0, 1, 5, 4),
          (2, 6, 7, 3), (0, 4, 6, 2), (1, 3, 7, 5))


def _box(sx: float, sy: float, sz: float, at=(0.0, 0.0, 0.0),
         color=None) -> Mesh:
    mesh = Mesh()
    x0, y0, z0 = at
    c = [QVector3D(x0 + dx * sx, y0 + dy * sy, z0 + dz * sz)
         for dz in (0, 1) for dy in (0, 1) for dx in (0, 1)]
    for q in _QUADS:
        f = mesh.add_face([c[k] for k in q])
        if color is not None:
            f.attrs["color"] = list(color)
    return mesh


def _part(mesh: Mesh, name: str, xform=None) -> Group:
    g = Group(mesh, name=name)
    g.xform = xform if xform is not None else QMatrix4x4()
    return g


def _stool() -> Group:
    """A seat and two identical legs, one of them splayed."""
    splay = QMatrix4x4()
    splay.translate(QVector3D(0.5, 0, 0))
    splay.rotate(10, 0, 1, 0)
    kids = [_part(_box(0.40, 0.40, 0.03, color=(0.6, 0.4, 0.2)), "seat"),
            _part(_box(0.04, 0.04, 0.58, color=(0.6, 0.4, 0.2)), "leg A"),
            _part(_box(0.04, 0.04, 0.58, color=(0.6, 0.4, 0.2)), "leg B",
                  splay)]
    stool = Group(name="stool")
    stool.adopt(kids)
    return stool


def test_a_board_measures_on_its_own_axes():
    size = part_size(part_points(_part(_box(0.8, 0.3, 0.018), "board")))
    assert size == pytest.approx((0.8, 0.3, 0.018), abs=1e-6)


def test_a_splayed_leg_measures_along_its_length():
    """Along the stool's axes the leg at 30° is a fat diagonal; along its
    own it is the stick it is."""
    turn = QMatrix4x4()
    turn.rotate(30, 0, 1, 0)
    leg = _part(_box(0.04, 0.04, 0.58), "leg", turn)
    assert part_size(part_points(leg)) == pytest.approx((0.58, 0.04, 0.04),
                                                        abs=1e-5)


def test_a_stepped_carcass_measures_on_the_models_axes():
    """A body with a plinth set back: the tilted box that holds it in a
    little less volume is not a size anyone cuts."""
    import numpy as np
    body = [(x, y, z) for x in (0, 0.442) for y in (-0.052, 0.56)
            for z in (0.085, 0.794)]
    plinth = [(x, y, z) for x in (0, 0.442) for y in (0.0, 0.56)
              for z in (0.0, 0.085)]
    size = part_size(np.array(body + plinth, dtype=float))
    assert size == pytest.approx((0.794, 0.612, 0.442), abs=1e-6)


def test_the_material_is_the_one_covering_most_of_the_part():
    mesh = _box(1.0, 1.0, 0.02, color=(1, 0, 0))
    small = [f for f in mesh.faces if f.area() < 0.1]
    for f in small:                        # the four thin edges: 0.08 m²
        f.attrs["color"] = [0, 0, 1]
    assert part_material(_part(mesh, "top")) == "#ff0000"


def test_an_opening_does_not_vote_for_the_material():
    """#742: the covering material is weighed by the surface that is
    really there — a 16 m² outline that is nearly all opening must not
    outvote a whole 1 m² face."""
    mesh = Mesh()
    red = mesh.add_face(
        [QVector3D(0, 0, 0), QVector3D(4, 0, 0), QVector3D(4, 4, 0),
         QVector3D(0, 4, 0)],
        [[QVector3D(0.05, 0.05, 0), QVector3D(0.05, 3.95, 0),
          QVector3D(3.95, 3.95, 0), QVector3D(3.95, 0.05, 0)]])
    red.attrs["color"] = [1, 0, 0]         # 16 m² gross, 0.79 m² net
    blue = mesh.add_face([QVector3D(5, 0, 0), QVector3D(6, 0, 0),
                          QVector3D(6, 1, 0), QVector3D(5, 1, 0)])
    blue.attrs["color"] = [0, 0, 1]        # 1 m², no opening
    assert part_material(_part(mesh, "panel")) == "#0000ff"


def test_a_part_inherits_its_containers_paint_on_default_faces():
    part = _part(_box(0.5, 0.5, 0.5), "cube")
    part.material = {"color": [0, 1, 0]}
    assert part_material(part) == "#00ff00"


def test_identical_parts_are_one_cut_list_line():
    rows = part_rows(_stool())
    lines = cut_list(rows)
    assert [(ln["qty"], ln["names"]) for ln in lines] == [
        (2, ["leg A", "leg B"]), (1, ["seat"])]
    text = cut_list_text(lines, lambda m: f"{m * 1000:.0f}",
                         ["Qty", "Parts", "Material", "L", "W", "T"])
    assert text.splitlines()[1] == "2\tleg A, leg B\t#996633\t580\t40\t40"


def test_the_material_can_be_left_out_of_the_match():
    """A Sweet Home 3D model paints every fragment with its own image, so
    two identical legs never share a material there."""
    stool = _stool()
    for f in stool.children[2].mesh.faces:
        f.attrs["color"] = [0.1, 0.1, 0.1]
    assert [ln["qty"] for ln in cut_list(part_rows(stool))] == [1, 1, 1]
    merged = cut_list(part_rows(stool), by_material=False)
    assert [ln["qty"] for ln in merged] == [2, 1]
    assert merged[0]["material"] == "#996633, #1a1a1a"


def test_names_sort_naturally():
    kids = [_part(_box(0.1, 0.1, 0.1), f"Piece {n}") for n in (10, 2, 1)]
    holder = Group()
    holder.adopt(kids)
    assert cut_list(part_rows(holder))[0]["names"] == [
        "Piece 1", "Piece 2", "Piece 10"]


def _window_with(native_host, group):
    win = native_host.window
    scene = win.viewport.scene
    scene.groups.append(group)
    scene.selection.clear()
    scene.selection.add(group)
    scene.version += 1
    return win, scene


def test_copy_cut_list_fills_the_clipboard(native_host, monkeypatch):
    win, scene = _window_with(native_host, _stool())
    from ingetrazo_render import chrome_sheet
    ctx = chrome_sheet.Ctx(win, win._doc.stand_in)
    from app_shell import native
    clipboard = {}
    monkeypatch.setattr(chrome_sheet, "in_shell", lambda: True)
    monkeypatch.setattr(native, "pasteboard_write", clipboard.update)
    native_host.menu("cutlist")
    sheet = chrome_sheet.current(win)
    sheet.build(ctx)
    sheet.act(ctx, "verb", ["copy"])
    lines = clipboard["public.utf8-plain-text"].decode().splitlines()
    assert len(lines) == 3 and lines[1].startswith("2\t")
    assert len(lines[0].split("\t")) == 6
    win._saved_version = scene.version


def test_part_sizes_resolve_the_millimetre_whatever_the_display_precision():
    """A 4 mm hinge leaf in a document shown to the centimetre read
    «0.03 × 0.02 × 0.00 m» — a part that looked like it had no thickness."""
    from core import units
    from core.scene import Scene
    scene = Scene()
    scene.units = {"length": "m", "precision": 2}
    units.bind_scene(scene)
    try:
        assert units.fmt_triple(0.0325, 0.024, 0.0043) == "0.03 × 0.02 × 0.00 m"
        assert units.fmt_triple(0.0325, 0.024, 0.0043, fine=True) == \
            "0.033 × 0.024 × 0.004 m"
        assert units.fmt_len_fine(0.0043) == "0.004 m"
        scene.units = {"length": "mm", "precision": 0}
        # 32.5 mm rounds up, as 0.0325 m does to 0.033 m above.
        assert units.fmt_triple(0.0325, 0.024, 0.0043, fine=True) == \
            "33 × 24 × 4 mm"
    finally:
        units.bind_scene(None)


def _sheet(sx: float, sy: float) -> Mesh:
    mesh = Mesh()
    pts = [QVector3D(0, 0, 0), QVector3D(sx, 0, 0), QVector3D(sx, sy, 0),
           QVector3D(0, sy, 0)]
    mesh.add_face(pts)
    mesh.add_face(list(reversed(pts)))
    return mesh


def test_a_part_with_no_thickness_is_a_surface_not_a_zero_board():
    """A skin or a pane drawn as one plane is kept — it is what the model
    shows — but the cut list must not list it as a 0-thick board."""
    from core.parts import is_surface
    holder = Group()
    holder.adopt([_part(_sheet(0.6, 0.135), "baseboard skin"),
                  _part(_box(0.6, 0.4, 0.018), "shelf")])
    rows = part_rows(holder)
    assert [is_surface(r["size"]) for r in rows] == [True, False]
    text = cut_list_text(cut_list(rows), lambda m: f"{m * 1000:.0f}",
                         ["Qty", "Parts", "Material", "L", "W", "T"])
    lines = text.splitlines()
    assert lines[1].split("\t")[-1] == "18"            # the shelf
    assert lines[2].split("\t")[-1] == "surface"       # the skin


def _copies(n=40):
    """``n`` placements of ONE definition (turned, scaled, moved), half of
    them painted as containers, under one container: what a scatter or a
    row of chairs is. The definition has a face of its own paint."""
    proto = _box(0.5, 0.5, 2.0)
    next(iter(proto.faces)).attrs["color"] = [0.2, 0.6, 0.2]
    kids = []
    for i in range(n):
        m = QMatrix4x4()
        m.translate(QVector3D(i * 1.5, 0, 0))
        m.rotate(i * 17.0, 0, 0, 1)
        m.scale(0.8 + 0.01 * i)
        k = _part(proto, f"copy {i}", m)
        if i % 2:
            k.material = {"color": [0.5, 0.3, 0.1]}
        kids.append(k)
    holder = Group(name="scatter")
    holder.adopt(kids)
    return holder


def test_copies_of_one_definition_are_read_once(monkeypatch):
    """A container of copies used to read the shared mesh once per copy,
    for the material and again for the points: 300 copies of a
    3 000-face tree froze the window ~14 s each time it was selected."""
    from core import parts
    holder = _copies(40)
    calls = {"net": 0}
    real = type(next(iter(holder.children[0].mesh.faces))).net_area

    def counting(self):
        calls["net"] += 1
        return real(self)

    monkeypatch.setattr(type(next(iter(holder.children[0].mesh.faces))),
                        "net_area", counting)
    rows = parts.part_rows(holder)
    assert len(rows) == 40
    assert calls["net"] == 6                     # one box's faces, once


def test_shared_reading_gives_the_rows_reading_one_by_one_gave():
    from core import parts
    holder = _copies(12)
    together = parts.part_rows(holder)
    alone = [parts.part_row(k) for k in holder.children]
    for a, b in zip(together, alone):
        assert a["material"] == b["material"]
        assert a["size"] == pytest.approx(b["size"], abs=1e-9)
        assert a["faces"] == b["faces"] == 6
    # Painted copies show their paint on the default faces; the others
    # show the definition's own face, the only paint they have.
    assert together[1]["material"] == "#804c1a"
    assert together[0]["material"] == "#339933"
    # And the size follows each copy's scale.
    assert together[0]["size"][0] == pytest.approx(1.6, abs=1e-6)
    assert together[10]["size"][0] == pytest.approx(1.8, abs=1e-6)


def test_parts_read_lazy_shared_prototypes_without_building(monkeypatch, tmp_path):
    import numpy as np
    from core import parts
    from core.group import iter_placements
    from core.mesh import DeferredMesh
    from core.scene import Scene
    from formats import igz
    from tests.test_deferred_mesh import _open
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 1)
    stool = _stool()
    # Both legs share a prototype but have different placements.
    stool.children[2].mesh = stool.children[1].mesh
    sc = Scene()
    sc.groups.append(stool)
    path = tmp_path / "parts.igz"
    igz.save_scene(sc, path)
    eager = _open(monkeypatch, path, False)
    expected = part_rows(eager.groups[0])
    lazy = _open(monkeypatch, path, True)
    assert any(type(g.mesh) is DeferredMesh for g, _ in iter_placements(lazy.groups[0]))
    monkeypatch.setattr(DeferredMesh, "_materialize", lambda self: pytest.fail(
        "parts inspection built editable topology"))
    assert lazy.groups[0].children[1].mesh is lazy.groups[0].children[2].mesh
    actual = part_rows(lazy.groups[0])
    for a, b in zip(actual, expected):
        for key in ("name", "material", "faces", "hidden"):
            assert a[key] == b[key]
        np.testing.assert_allclose(a["size"], b["size"], rtol=1e-12, atol=1e-12)
