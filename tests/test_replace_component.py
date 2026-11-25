# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Select ▸ All Instances and Replace Component…: select every copy of a
component and swap them for another definition, each copy keeping its
place, turn and scale, its layer, paint and visibility."""
from __future__ import annotations

import pytest
from tests.test_library_sheet import win  # noqa: F401
from built_core import Matrix4 as QMatrix4x4, Vec3 as V

from core.group import Group, copy_group, world_mesh
from core.history import History, ReplaceComponentCommand
from core.mesh import Mesh
from core.scene import Scene
from built_core import same_definition
from tests.shell_isolation import native_question_host, restore_shell_patches  # noqa: F401


def _quad(size=1.0, z=0.0):
    m = Mesh()
    m.add_face([V(0, 0, z), V(size, 0, z), V(size, size, z), V(0, size, z)])
    return m


def _component(mesh, name, dx=0.0, turn=0.0, scale=1.0):
    g = Group(mesh, name=name)
    xf = QMatrix4x4()
    xf.translate(dx, 0.0, 0.0)
    xf.rotate(turn, 0.0, 0.0, 1.0)
    xf.scale(scale)
    g.xform = xf
    return g


def _points(group):
    return sorted((round(v.position.x(), 6), round(v.position.y(), 6),
                   round(v.position.z(), 6))
                  for v in world_mesh(group).vertices)


def _model(moved=False):
    """Three copies of «Silla», two of «Mesa», a classic group and a group
    of groups (a matrix, but not a component). ``moved``: the caller puts
    them in another scene, so this one lets go of them (a placement has one
    parent)."""
    scene = Scene()
    silla = _component(_quad(1.0), "Silla", dx=0.0)
    sillas = [silla, copy_group(silla, V(5, 0, 0)),
              copy_group(silla, V(10, 0, 0))]
    mesa = _component(_quad(2.0, z=1.0), "Mesa", dx=20.0)
    mesas = [mesa, copy_group(mesa, V(5, 0, 0))]
    classic = Group(_quad(3.0), name="Muro")
    holder = _component(_quad(0.5), "Caja", dx=-5.0)
    holder.component = False
    if not moved:
        scene.groups.extend(sillas + mesas + [classic, holder])
    return scene, History(scene), sillas, mesas, classic, holder


# --- Select ▸ All Instances ------------------------------------------------

def test_all_instances_takes_every_copy_and_nothing_else():
    scene, _h, sillas, mesas, classic, holder = _model()
    got = same_definition(scene, [sillas[1]])
    assert set(got) == set(sillas)
    assert not set(got) & {*mesas, classic, holder}


def test_all_instances_of_two_definitions_takes_both_families():
    scene, _h, sillas, mesas, _c, _holder = _model()
    assert set(same_definition(scene, [sillas[0], mesas[1]])) == {
        *sillas, *mesas}


def test_all_instances_ignores_groups_and_loose_geometry():
    scene, _h, _s, _m, classic, holder = _model()
    face = scene.mesh.add_face([V(0, 0, 5), V(1, 0, 5), V(1, 1, 5)])
    assert same_definition(scene, [classic, holder, face]) == []


def test_all_instances_answers_from_the_open_group_only():
    """Inside a group, only the copies at that level — like Select All."""
    scene, _h, sillas, _m, _c, _holder = _model()
    inner = _component(_quad(1.0), "Pata")
    inner_copy = copy_group(inner, V(2, 0, 0))
    box = Group(Mesh(), name="Conjunto")
    box.adopt([inner, inner_copy])
    box.component = False
    scene.groups.append(box)
    scene.begin_group_edit(box)
    try:
        assert set(same_definition(scene, [inner])) == {inner, inner_copy}
        assert same_definition(scene, [sillas[0]]) == []
    finally:
        scene.end_group_edit()


# --- Replace Component -----------------------------------------------------

def test_replace_swaps_the_definition_and_keeps_the_placement():
    scene, hist, sillas, mesas, _c, _h = _model()
    target = sillas[1]
    xf = QMatrix4x4(target.xform)
    hist.execute(ReplaceComponentCommand([target], mesas[0]))
    assert target.mesh is mesas[0].mesh
    assert target.name == "Mesa"
    assert target.xform == xf                       # same place
    # Mesa's own origin lands on the chair's: x 5..7, y 0..2, z 1.
    assert _points(target) == sorted([(5.0, 0.0, 1.0), (7.0, 0.0, 1.0),
                                      (7.0, 2.0, 1.0), (5.0, 2.0, 1.0)])
    # the other chairs are untouched
    assert sillas[0].mesh is sillas[2].mesh and sillas[0].name == "Silla"


def test_replace_follows_the_targets_turn_and_scale():
    scene = Scene()
    hist = History(scene)
    a = _component(_quad(1.0), "A", dx=3.0, turn=90.0, scale=2.0)
    b = _component(_quad(1.0, z=0.5), "B", dx=50.0)
    scene.groups.extend([a, b])
    hist.execute(ReplaceComponentCommand([a], b))
    # B's unit square, turned 90° and doubled at x = 3
    assert _points(a) == sorted([(3.0, 0.0, 1.0), (3.0, 2.0, 1.0),
                                 (1.0, 2.0, 1.0), (1.0, 0.0, 1.0)])


def test_replace_keeps_what_belongs_to_the_copy():
    scene, hist, sillas, mesas, _c, _h = _model()
    t = sillas[0]
    t.layer = "Mobiliario"
    t.material = {"color": (1.0, 0.0, 0.0)}
    t.hidden = True
    uid = t.uid
    mesas[0].ifc = {"class": "IfcFurniture", "name": "Mesa"}
    hist.execute(ReplaceComponentCommand([t], mesas[0]))
    assert t.layer == "Mobiliario"
    assert t.material == {"color": (1.0, 0.0, 0.0)}
    assert t.hidden is True
    assert t.uid == uid
    assert t.ifc == {"class": "IfcFurniture", "name": "Mesa"}
    assert t.ifc is not mesas[0].ifc                # not aliased


def test_replace_all_instances_then_undo_and_redo():
    scene, hist, sillas, mesas, _c, _h = _model()
    before = [(_points(g), g.name) for g in sillas]
    v0 = scene.version
    hist.execute(ReplaceComponentCommand(
        same_definition(scene, [sillas[0]]), mesas[1]))
    assert scene.version > v0
    assert all(g.mesh is mesas[0].mesh for g in sillas)
    assert {g.name for g in sillas} == {"Mesa"}
    assert set(same_definition(scene, [mesas[0]])) == {*sillas, *mesas}
    hist.undo()
    assert [(_points(g), g.name) for g in sillas] == before
    assert sillas[0].mesh is sillas[1].mesh is sillas[2].mesh
    assert sillas[0].mesh is not mesas[0].mesh
    hist.redo()
    assert all(g.mesh is mesas[0].mesh for g in sillas)


def test_replace_skips_its_own_definition_and_non_components():
    scene, _h, sillas, mesas, classic, holder = _model()
    cmd = ReplaceComponentCommand([*mesas, classic, holder, sillas[0]],
                                  mesas[0])
    assert cmd.targets == [sillas[0]]


def test_replaced_copies_share_the_sources_subgroups():
    """A source with subgroups: the replaced copy holds them SHARED, so an
    edit inside any copy reaches every one, as copies of a component do."""
    scene = Scene()
    hist = History(scene)
    banca = _component(_quad(1.0), "Banca", dx=0.0)
    banca.adopt([_component(_quad(0.2, z=0.5), "Respaldo")])
    target = _component(_quad(1.0), "Silla", dx=8.0)
    scene.groups.extend([banca, target])
    hist.execute(ReplaceComponentCommand([target], banca))
    assert len(target.children) == 1
    assert target.children[0].mesh is banca.children[0].mesh
    assert target.children[0] is not banca.children[0]
    assert target.children[0].uid != banca.children[0].uid
    hist.undo()
    assert target.children == [] and target.name == "Silla"


def test_replacement_survives_the_igz(tmp_path):
    from formats import igz
    scene, hist, sillas, mesas, _c, _h = _model()
    hist.execute(ReplaceComponentCommand(sillas, mesas[0]))
    path = tmp_path / "r.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    mesa_groups = [g for g in back.groups if g.name == "Mesa"]
    assert len(mesa_groups) == 5
    assert len({id(g.mesh) for g in mesa_groups}) == 1
    assert sorted(_points(g) for g in mesa_groups) == sorted(
        _points(g) for g in [*sillas, *mesas])


# --- the window --------------------------------------------------------------

# --- the tray's «In model» list ----------------------------------------------

def _provider(win):
    from built_render import wgpu_library as library
    return library.ComponentsProvider(), library.Ctx(win, win.view)


def _rows(win):
    provider, ctx = _provider(win)
    return {r["name"]: r for r in provider.items(ctx) if r["key"].startswith("cm:")}


def _run(win, name, verb):
    provider, ctx = _provider(win)
    row = _rows(win)[name]
    assert verb in row["verbs"]
    provider.run(ctx, verb, row)


def test_the_in_model_list_selects_and_replaces_from_its_menu(native_components, monkeypatch):
    win, scene, chairs, tables = native_components
    monkeypatch.setattr(win, "_ask_keep_orphans", lambda _: False)
    assert "replace" not in _rows(win)["Silla"]["verbs"]
    _run(win, "Silla", "select")
    assert set(scene.selection) == set(chairs)
    _run(win, "Mesa", "replace")
    assert all(chair.mesh is tables[0].mesh for chair in chairs)
    assert win.viewport.history.undo()
    assert all(chair.name == "Silla" for chair in chairs)


def test_its_own_row_does_not_offer_to_replace_the_selection(native_components):
    win, scene, chairs, tables = native_components
    scene.select(chairs)
    assert "replace" not in _rows(win)["Silla"]["verbs"]
    assert "replace" in _rows(win)["Mesa"]["verbs"]


# --- Keep in File: definitions with no copy placed ----------------------------

def test_keep_in_file_survives_deleting_every_copy_and_undoes():
    from core.history import KeepComponentCommand
    from built_core.kept_components import kept_template
    scene, hist, sillas, _m, _c, _h = _model()
    proto = sillas[0].mesh
    sillas[0].layer = "Mobiliario"
    hist.execute(KeepComponentCommand(sillas[0]))
    t = kept_template(scene, proto)
    assert t is not None and t.mesh is proto and t.name == "Silla"
    assert t not in scene.groups                    # never drawn
    assert t.xform == QMatrix4x4() and t.layer is None
    hist.execute(KeepComponentCommand(sillas[1]))   # already kept: no-op
    assert len(scene.kept_components) == 1
    for g in sillas:
        scene.groups.remove(g)
    assert kept_template(scene, proto) is t
    hist.undo()
    assert len(scene.kept_components) == 1
    hist.undo()
    assert scene.kept_components == []
    hist.redo()
    assert scene.kept_components == [t]


def test_stop_keeping_takes_it_out_and_undo_puts_it_back():
    from core.history import KeepComponentCommand
    scene, hist, sillas, mesas, _c, _h = _model()
    hist.execute(KeepComponentCommand(sillas[0]))
    hist.execute(KeepComponentCommand(mesas[0]))
    silla_t = scene.kept_components[0]
    hist.execute(KeepComponentCommand(silla_t, keep=False))
    assert [t.name for t in scene.kept_components] == ["Mesa"]
    hist.undo()
    assert scene.kept_components[0] is silla_t


def test_an_edit_to_the_definition_reaches_the_kept_template():
    from core.history import KeepComponentCommand
    scene, hist, sillas, _m, _c, _h = _model()
    hist.execute(KeepComponentCommand(sillas[0]))
    t = scene.kept_components[0]
    scene.begin_group_edit(sillas[1])
    scene.mesh.vertices[0].position = V(0, 0, 3)
    scene.end_group_edit()
    assert t.mesh is sillas[0].mesh
    assert any(round(v.position.z(), 6) == 3.0 for v in t.mesh.vertices)


def test_kept_definitions_survive_the_igz_and_share_with_copies(tmp_path):
    from core.history import KeepComponentCommand
    from formats import igz
    scene, hist, sillas, mesas, _c, _h = _model()
    hist.execute(KeepComponentCommand(sillas[0]))   # still placed
    hist.execute(KeepComponentCommand(mesas[0]))
    for g in mesas:                                 # none placed any more
        scene.groups.remove(g)
    path = tmp_path / "k.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    names = sorted(t.name for t in back.kept_components)
    assert names == ["Mesa", "Silla"]
    by_name = {t.name: t for t in back.kept_components}
    assert len(by_name["Mesa"].mesh.faces) == 1
    assert not any(g.name == "Mesa" for g in back.groups)
    sillas_back = [g for g in back.groups if g.name == "Silla"]
    assert all(g.mesh is by_name["Silla"].mesh for g in sillas_back)
    back.clear()
    assert back.kept_components == []


def test_a_kept_definition_with_no_copy_can_replace_and_be_placed():
    from core.history import KeepComponentCommand
    from built_core.kept_components import place_copy
    scene, hist, sillas, mesas, _c, _h = _model()
    hist.execute(KeepComponentCommand(mesas[0]))
    for g in mesas:
        scene.groups.remove(g)
    t = scene.kept_components[0]
    hist.execute(ReplaceComponentCommand(sillas[:1], t))
    assert sillas[0].mesh is t.mesh and sillas[0].name == "Mesa"
    xf = QMatrix4x4()
    xf.translate(30.0, 0.0, 0.0)
    g = place_copy(t, xf)
    assert g.mesh is t.mesh and g.is_component() and g.xform == xf
    assert g is not t and g.uid != t.uid


def test_the_list_shows_a_kept_definition_with_no_copy(native_components, monkeypatch):
    from core.history import KeepComponentCommand
    win, scene, chairs, tables = native_components
    monkeypatch.setattr(win, "_ask_keep_orphans", lambda _: False)
    win.viewport.history.execute(KeepComponentCommand(tables[0]))
    for group in tables:
        scene.groups.remove(group)
    scene.version += 1
    scene.select(chairs)
    kept = scene.kept_components[0]
    row = _rows(win)["Mesa"]
    assert row["uses"] == 0 and row["category"] == "Kept, not placed"
    assert "select" not in row["verbs"] and "unkeepfile" in row["verbs"]
    _run(win, "Mesa", "replace")
    assert all(chair.mesh is kept.mesh for chair in chairs)
    _run(win, "Mesa", "apply")
    assert win.viewport.active_tool._group.mesh is kept.mesh
    win.viewport.set_active_tool(None)
    assert win.viewport.history.undo()
    _run(win, "Mesa", "unkeepfile")
    assert not scene.kept_components and "Mesa" not in _rows(win)


def _painted_pixels(pm):
    return sum(alpha > 0 for alpha in pm.rgba()[3::4])


def test_a_thumbnail_is_drawn_once_and_again_when_the_geometry_changes():
    from built_render import component_thumbnail
    size = 40
    _s, _h, sillas, _m, _c, _holder = _model()
    pm = component_thumbnail(sillas[0])
    assert pm.width == pm.height == size
    assert _painted_pixels(pm) > size * size // 10
    assert component_thumbnail(sillas[1]) is pm               # same definition: cached
    sillas[0].mesh.vertices[0].position = V(0, 0, 4)
    assert component_thumbnail(sillas[0]) is not pm           # its geometry changed


def test_every_copy_draws_the_same_wherever_it_stands():
    from built_render import component_signature
    a = _component(_quad(1.0), "A", dx=0.0)
    b = copy_group(a, V(30, 0, 0))
    b.xform.rotate(45.0, 0.0, 0.0, 1.0)
    assert component_signature(a) == component_signature(b)


def test_an_empty_definition_gives_a_blank_icon():
    from built_render import component_thumbnail
    g = _component(Mesh(), "Nada")
    assert _painted_pixels(component_thumbnail(g)) == 0


def _component_sheet(win):
    from built_render.wgpu_library import open_library
    from tests.test_library_sheet import frame, tiles
    open_library(win, "components", scope="s:model")
    return [r for r in tiles(frame(win)) if r["id"].startswith("cm:")]


def test_the_list_rows_carry_a_thumbnail(win):
    from tests.test_library_sheet import act
    groups = [_component(_quad(i + 1), name) for i, name in enumerate(("A", "B"))]
    win.viewport.scene.groups.extend(groups)
    win.viewport.scene.version += 1
    tiles = _component_sheet(win)
    assert len(tiles) == 2
    act(win, "component-previews", *(r["icon"] for r in tiles))
    images, = win.view.renderer.last("ui_set_icons")
    assert {key for key, _, _, _ in images} == {r["icon"] for r in tiles}
    assert all(w == h == 96 and any(pixels[3::4]) for _, w, h, pixels in images)


def test_a_long_list_offers_a_name_filter_that_survives_a_refresh(win, monkeypatch):
    from tests.test_library_sheet import act, frame, tiles
    from built_core import component_library as library
    monkeypatch.setattr(library, "index", lambda refresh: [])
    names = ["Silla", "Sillón", "Mesa", "Lámpara", "Puerta P1", "Puerta P2", "Ventana V1", "Árbol"]
    scene = win.viewport.scene
    scene.groups.extend(_component(_quad(), n) for n in names)
    scene.version += 1
    _component_sheet(win)
    act(win, "search", "puerta")
    for _ in range(2):
        assert sorted(r["label"] for r in tiles(frame(win)) if r["id"].startswith("cm:")) == ["Puerta P1", "Puerta P2"]
        scene.version += 1


def test_short_component_lists_use_the_shared_library_search(win):
    from tests.test_library_sheet import frame
    win.viewport.scene.groups.append(_component(_quad(), "Table"))
    win.viewport.scene.version += 1
    assert any(r["label"] == "Table" for r in _component_sheet(win))
    assert frame(win)["search"]


def test_purge_keeps_what_only_a_kept_definition_wears():
    from core.history import KeepComponentCommand
    from core.purge import used_layers, used_materials
    scene, hist, _s, mesas, _c, _h = _model()
    for f in mesas[0].mesh.faces:
        f.attrs["mat"] = "Roble"
    inner = _component(_quad(0.3), "Cajón")
    inner.layer = "Cajones"
    mesas[0].children = [inner]
    hist.execute(KeepComponentCommand(mesas[0]))
    for g in mesas:
        scene.groups.remove(g)
    assert "Roble" in used_materials(scene)
    assert "Cajones" in used_layers(scene)


# --- the four follow-ups -------------------------------------------------------

@pytest.fixture
def native_components(native_question_host):
    win = native_question_host.window
    scene = win.viewport.scene
    _s, _h, sillas, mesas, classic, holder = _model(moved=True)
    scene.groups.extend(sillas + mesas + [classic, holder])
    scene.version += 1
    return win, scene, sillas, mesas


def test_orphaned_by_names_only_definitions_left_with_no_copy():
    from core.history import KeepComponentCommand
    from built_core.kept_components import orphaned_by
    scene, hist, sillas, mesas, _c, _h = _model()
    assert orphaned_by(scene, sillas[:2]) == []          # one chair stays
    assert [g.name for g in orphaned_by(scene, sillas + mesas[:1])] == [
        "Silla"]
    hist.execute(KeepComponentCommand(sillas[0]))
    assert orphaned_by(scene, sillas) == []              # kept: not lost


def test_a_copy_nested_elsewhere_still_counts_as_a_copy():
    from built_core.kept_components import orphaned_by
    scene, _h, sillas, _m, _c, _holder = _model()
    box = Group(Mesh(), name="Comedor")
    box.adopt([copy_group(sillas[0], V(0, 5, 0))])
    box.component = False
    scene.groups.append(box)
    assert orphaned_by(scene, sillas) == []


def test_replacing_the_last_copies_can_keep_them_in_one_undo(native_components, monkeypatch):
    win, scene, sillas, mesas = native_components
    asked = []
    monkeypatch.setattr(win, "_ask_keep_orphans",
                        lambda o: asked.append(
                            [g.name for g in o]) or True)
    scene.selection.clear()
    scene.selection.update(sillas)
    assert win.replace_selection_with(mesas[0]) == 3
    assert asked == [["Silla"]]          # asked before swapping
    kept = scene.kept_components
    assert [t.name for t in kept] == ["Silla"]
    assert all(g.mesh is mesas[0].mesh for g in sillas)
    win.viewport.history.undo()                 # both, in one step
    assert scene.kept_components == []
    assert all(g.name == "Silla" for g in sillas)


def test_cancel_in_the_question_replaces_nothing(native_components, monkeypatch):
    win, scene, sillas, mesas = native_components
    monkeypatch.setattr(win, "_ask_keep_orphans",
                        lambda o: None)
    scene.selection.clear()
    scene.selection.update(sillas)
    assert win.replace_selection_with(mesas[0]) == 0
    assert all(g.name == "Silla" for g in sillas)
    assert not win.viewport.history.undo_stack


def test_no_question_while_a_copy_remains(native_components, monkeypatch):
    win, scene, sillas, mesas = native_components
    def boom(o):
        raise AssertionError("should not ask")
    monkeypatch.setattr(win, "_ask_keep_orphans", boom)
    scene.selection.clear()
    scene.selection.update(sillas[:2])
    assert win.replace_selection_with(mesas[0]) == 2


def test_rename_reaches_every_copy_and_the_kept_template(native_components, monkeypatch):
    from built_render import chrome_forms
    from core.history import KeepComponentCommand
    win, scene, sillas, mesas = native_components
    box = Group(Mesh(), name="Comedor")
    nested = copy_group(sillas[0], V(0, 5, 0))
    box.adopt([nested])
    box.component = False
    scene.groups.append(box)
    win.viewport.history.execute(KeepComponentCommand(sillas[0]))
    win.rename_component(sillas[1])
    form = chrome_forms.current(win)
    form.receive("text", "Silla Bistró")
    form.press("ok")
    assert {g.name for g in sillas} == {"Silla Bistró"}
    assert nested.name == "Silla Bistró"
    assert scene.kept_components[0].name == "Silla Bistró"
    assert {g.name for g in mesas} == {"Mesa"}
    win.viewport.history.undo()
    assert {g.name for g in sillas} == {"Silla"}
    assert scene.kept_components[0].name == "Silla"


def test_rename_is_in_the_list_menu(native_components):
    from built_render import wgpu_library as library
    win, _, _, _ = native_components
    provider = library.ComponentsProvider()
    chair = next(e for e in provider.items(library.Ctx(win, win.view)) if e["name"] == "Silla")
    assert "rename" in chair["verbs"]


def test_components_from_another_file_arrive_kept_and_placeable(tmp_path, native_host, monkeypatch):
    from formats import igz
    from core.history import KeepComponentCommand
    # the «library» file: two placed definitions, one kept, a plain group
    lib, hist, sillas, mesas, _c, _h = _model()
    from core.materials import Material
    lib.materials["Brass"] = Material("Brass", color=(0.9, 0.8, 0.1))
    lamp = _component(_quad(0.3), "Lámpara")
    for f in lamp.mesh.faces:
        f.attrs.update(lib.materials["Brass"].face_attrs())
        f.attrs["layer"] = "Lighting"
    lib.groups.append(lamp)
    hist.execute(KeepComponentCommand(lamp))
    lib.groups.remove(lamp)
    path = tmp_path / "biblioteca.igz"
    igz.save_scene(lib, path)

    win = native_host.window
    scene = win.viewport.scene
    from built_render import wgpu_library as library
    from tests.test_library_sheet import Renderer, act, frame
    from built_core.i18n import tr
    from built_render.shell_files import ShellFileDialog
    from built_render import chrome_dialogs
    monkeypatch.setattr(chrome_dialogs, "warning", lambda _, title, msg: pytest.fail(f"{title}: {msg}"))
    monkeypatch.setattr(ShellFileDialog, "getOpenFileName", lambda *_: (str(path), ""))
    monkeypatch.setattr(win.view, "renderer", Renderer())
    monkeypatch.setattr(win.view, "_ui_attached", True, raising=False)
    assert library.open_library(win, "components", scope="s:model")
    blocks = frame(win)["content"]["blocks"]
    buttons = {b["id"]: b["label"] for block in blocks if block["kind"] == "buttons"
               for b in block["buttons"]}
    assert buttons["typeact:import"] == tr("Components from File…")
    assert "typeact:purge" in buttons
    act(win, "verb", "typeact:import")
    assert len(scene.kept_components) == 3                       # Silla, Mesa and Lámpara
    names = sorted(t.name for t in scene.kept_components)
    assert "Lámpara" in names and "Silla" in names and "Mesa" in names
    assert "Muro" not in names and "Caja" not in names   # groups
    assert not [g for g in scene.groups if g.name in names]
    lamp_t = next(t for t in scene.kept_components
                  if t.name == "Lámpara")
    assert lamp_t.mesh.faces[0].attrs["mat"] == "Brass"
    assert scene.layer_by_key("Lighting") is not None and "Brass" in scene.materials
    win.viewport.history.undo()
    assert not scene.kept_components and scene.layer_by_key("Lighting") is None
    assert "Brass" not in scene.materials
    win.viewport.history.redo()
    assert len(scene.kept_components) == 3 and "Brass" in scene.materials


def test_a_file_without_components_changes_nothing(tmp_path, native_host):
    from formats import igz
    plain = Scene()
    plain.groups.append(Group(_quad(2.0), name="Muro"))
    path = tmp_path / "muro.igz"
    igz.save_scene(plain, path)
    win = native_host.window
    from built_render import import_kept_components
    assert import_kept_components(win, path) == 0
    assert win.viewport.scene.kept_components == []
    assert not win.viewport.history.undo_stack


def test_clipped_component_list_does_not_build_thumbnails(win, monkeypatch):
    from tests.test_library_sheet import act
    win.viewport.scene.groups.append(_component(_quad(), "Table"))
    win.viewport.scene.version += 1
    calls = []
    import built_core
    plan = built_core.plan_of
    monkeypatch.setattr(built_core, "plan_of", lambda mesh: (calls.append(mesh), plan(mesh))[1])
    rows = _component_sheet(win)
    assert not calls
    act(win, "component-previews")
    assert not calls
    act(win, "component-previews", rows[0]["icon"])
    assert calls == [win.viewport.scene.groups[-1].mesh]
