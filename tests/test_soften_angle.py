# SPDX-License-Identifier: GPL-3.0-or-later
"""Edit ▸ Soften / Smooth Edges…: angle smoothing as one undo step, with
the soft flag an import sets (formats.fuse.soften_smooth_edges), shaded
smooth in the GL viewport and in the Metal view."""
import numpy as np
import pytest
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V

from core.group import Group, copy_group
from core.history import History
from core.mesh import Mesh
from core.scene import Scene
from core.soften import SoftenByAngleCommand, selected_edges
from tests.test_viewport_smooth_shading import _cylinder, _stub

from tests.shell_isolation import native_window, restore_shell_patches  # noqa: F401

SEAMS = 12          # the cylinder's side seams: 30° apart


def _soft(mesh):
    return sum(bool(e.soft) for e in mesh.edges)


def _scene_with(group):
    scene = Scene()
    scene.groups = [group]
    scene.selection = {group}
    return scene


def test_angle_softens_cylinder_sides_keeps_caps_and_undoes():
    group = Group(_cylinder(Mesh(), soft=False))
    scene = _scene_with(group)
    before = list(group.mesh.edges)
    hist = History(scene)
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40))
    assert hist.last_error is None
    assert _soft(group.mesh) == SEAMS            # the 90° cap rims stay hard
    assert list(group.mesh.edges) == before      # no topology change
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 10))
    assert _soft(group.mesh) == 0
    hist.undo()
    assert _soft(group.mesh) == SEAMS
    hist.undo()
    assert _soft(group.mesh) == 0
    hist.redo()
    assert _soft(group.mesh) == SEAMS


def test_the_angle_reads_like_the_import_whatever_the_winding():
    """A side face drawn the other way round still smooths, as the import
    pass (abs of the normals' dot) smooths it."""
    from formats.fuse import soften_smooth_edges
    mesh = _cylinder(Mesh(), soft=False)
    mesh.faces[3].loop.reverse()
    mesh.mark_modified()
    imported = _cylinder(Mesh(), soft=False)
    imported.faces[3].loop.reverse()
    imported.mark_modified()
    soften_smooth_edges(imported, cos_threshold=0.85)       # ~31.8°
    group = Group(mesh)
    scene = _scene_with(group)
    History(scene).execute(SoftenByAngleCommand(selected_edges(scene), 31.8))
    assert _soft(mesh) == _soft(imported) == SEAMS


def test_a_material_boundary_keeps_its_line_unless_asked():
    mesh = _cylinder(Mesh(), soft=False)
    mesh.faces[0].attrs["color"] = (1.0, 0.0, 0.0)
    group = Group(mesh)
    scene = _scene_with(group)
    hist = History(scene)
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40))
    assert _soft(mesh) == SEAMS - 2
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40,
                                      materials=True))
    assert _soft(mesh) == SEAMS


def test_face_selection_does_not_soften_its_outer_boundary():
    scene = Scene()
    _cylinder(scene.mesh, soft=False)
    scene.selection = set(scene.mesh.faces[:2])
    targets = selected_edges(scene)
    assert len(targets[scene.mesh]) == 1
    History(scene).execute(SoftenByAngleCommand(targets, 180))
    assert _soft(scene.mesh) == 1


def test_nested_definitions_are_processed_once_and_hidden_flags_survive():
    child = Group(_cylinder(Mesh(), soft=False))
    child.xform = QMatrix4x4()
    parent = Group()
    parent.adopt([child, copy_group(child)])
    scene = _scene_with(parent)
    edge = child.mesh.edges[0]
    edge.hidden = True
    targets = selected_edges(scene)
    assert len(targets[child.mesh]) == len(child.mesh.edges)
    History(scene).execute(SoftenByAngleCommand(targets, 40))
    assert edge.hidden
    assert parent.children[0].mesh is parent.children[1].mesh
    assert _soft(child.mesh) == SEAMS


def test_locked_and_linked_groups_are_left_alone(tmp_path):
    from core import links
    from formats import igz
    locked = Group(_cylinder(Mesh(), soft=False))
    locked.locked = True
    src = Scene()
    src.groups.append(Group(_cylinder(Mesh(), soft=False), name="drum"))
    path = tmp_path / "Site.igz"
    igz.save_scene(src, path)
    scene = Scene()
    link = links.make_link(scene, path, host_path=tmp_path / "host.igz")
    scene.groups = [locked, link]
    scene.selection = {locked, link, link.children[0]}
    assert selected_edges(scene) == {}


def test_smoothing_invalidates_the_cached_chunk_and_undo_restores_it():
    group = Group(_cylinder(Mesh(), soft=False))
    scene = _scene_with(group)
    vp = _stub(scene)
    before = vp._group_chunk(group)["vcol"]
    hist = History(scene)
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40))
    after = vp._group_chunk(group)["vcol"]
    assert before != after
    hist.undo()
    assert vp._group_chunk(group)["vcol"] == before


def test_coplanar_and_unsoften_controls():
    scene = Scene()
    a = scene.mesh.add_face([V(0, 0, 0), V(1, 0, 0), V(1, 1, 0)])
    b = scene.mesh.add_face([V(0, 0, 0), V(1, 1, 0), V(0, 1, 0)])
    scene.selection = {a, b}
    hist = History(scene)
    targets = selected_edges(scene)
    hist.execute(SoftenByAngleCommand(targets, 1, coplanar=False))
    assert _soft(scene.mesh) == 0
    hist.execute(SoftenByAngleCommand(targets, 1))
    assert _soft(scene.mesh) == 1
    hist.execute(SoftenByAngleCommand(targets, harden=True))
    assert _soft(scene.mesh) == 0
    hist.undo()
    assert _soft(scene.mesh) == 1


def test_the_command_replays_by_key_across_other_undos():
    """Undo of an earlier mesh edit restores the mesh's elements by key:
    the smoothing still replays on them."""
    from core.edits import build_add_edge
    scene = Scene()
    _cylinder(scene.mesh, soft=False)
    hist = History(scene)
    scene.selection = set(scene.mesh.faces)
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40))
    assert _soft(scene.mesh) == SEAMS
    hist.execute(build_add_edge(scene, V(5, 5, 0), V(6, 5, 0)))
    hist.undo()
    hist.undo()
    assert _soft(scene.mesh) == 0
    hist.redo()
    assert _soft(scene.mesh) == SEAMS


def test_metal_rebakes_the_smoothed_definition_without_a_repack(monkeypatch):
    """Both copies of a component shade smooth from one rebake of their
    definition; undo brings the facets back. No full pack, no rescan of
    the placements."""
    from tests.test_wgpu_updates import Renderer, assert_matches_fresh_pack
    from views.wgpu_scene import pack_scene
    from views.wgpu_updates import InstanceUpdates
    mesh = _cylinder(Mesh(), soft=False)
    a, b = Group(mesh, name="a"), Group(mesh, name="b")
    a.xform, b.xform = QMatrix4x4(), QMatrix4x4()
    b.xform.translate(5, 0, 0)
    scene = Scene(groups=[a, b])
    scene.selection = {a}
    updates = InstanceUpdates(scene, pack_scene(scene))
    hist = History(scene)
    updates.observe(hist)
    flat = updates.packed.vertices.copy()
    bakes = []
    monkeypatch.setattr(
        "views.wgpu_updates.pack_scene",
        lambda s, **kw: (bakes.append(len(s.groups)), pack_scene(s, **kw))[1])

    def settle():
        bakes.clear()
        delta = updates.diff(scene)
        assert delta is not None
        updates.apply(*delta, Renderer())
        assert updates.last_local
        assert bakes == [1]                         # one definition, once
        assert_matches_fresh_pack(updates, scene)

    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40))
    settle()
    code = int(updates.packed.instances["info"][updates.resident.row(a.uid), 0])
    smooth = updates.packed.vertices.copy()
    assert code == int(updates.packed.instances["info"][updates.resident.row(b.uid), 0])
    assert not np.array_equal(flat, smooth)       # the normals moved
    hist.undo()
    settle()
    hist.redo()
    settle()


def test_dialog_cancel_and_accept_use_one_undo_step(native_window, monkeypatch):
    from ingetrazo_render import chrome_forms
    win = native_window
    forms = []
    monkeypatch.setattr(chrome_forms, "show", lambda _, form: forms.append(form))
    scene = win.viewport.scene
    group = Group(_cylinder(Mesh(), soft=False))
    scene.groups = [group]
    scene.selection = {group}
    win._on_soften_edges()
    forms.pop().press("cancel")
    assert _soft(group.mesh) == 0
    assert not win.viewport.history.undo_stack

    win._on_soften_edges()
    form = forms.pop()
    form.receive("angle", "40")
    form.press("ok")
    assert _soft(group.mesh) == SEAMS
    assert len(win.viewport.history.undo_stack) == 1
    win.viewport.history.undo()
    assert _soft(group.mesh) == 0


def test_smoothing_inside_a_component_reaches_its_copies(native_window, monkeypatch):
    """Inside an open component the command runs on its world copy; the
    share-back carries the soft flags to every copy, one undo step."""
    from ingetrazo_render import chrome_forms
    mesh = _cylinder(Mesh(), soft=False)
    a, b = Group(mesh, name="a"), Group(mesh, name="b")
    a.xform, b.xform = QMatrix4x4(), QMatrix4x4()
    b.xform.translate(5, 0, 0)
    win = native_window
    forms = []
    monkeypatch.setattr(chrome_forms, "show", lambda _, form: forms.append(form))
    vp = win.viewport
    vp.scene.groups.extend([a, b])
    vp.scene.version += 1
    steps = len(vp.history.undo_stack)
    vp.begin_group_edit(a)
    vp.scene.selection = set(vp.scene.mesh.faces)

    win._on_soften_edges()
    form = forms.pop()
    form.receive("angle", "40")
    form.press("ok")
    vp.end_group_edit()
    assert a.mesh is b.mesh is mesh
    assert _soft(mesh) == SEAMS
    assert len(vp.history.undo_stack) == steps + 1
    vp.history.undo()
    assert _soft(mesh) == 0


def test_the_dialog_softens_across_materials_by_default_as_.skp():
    """the modeller's Soften Edges has no material rule: the dialog starts
    with «Soften between different materials» ticked."""
    from ingetrazo_render import OperationForm
    options = OperationForm.soften("30").values()
    assert options["materials"] and options["coplanar"]


# ---- «Smooth normals» (the modeller's Soften Edges): soft, and smooth or not ----

def _flat(mesh):
    return sum(bool(e.soft and e.flat) for e in mesh.edges)


def test_smooth_normals_off_hides_the_lines_and_keeps_the_facets():
    """Off, the edges go soft (no line) but every face keeps its flat
    shade in the GL view; on (the default), they shade smooth."""
    hard = Group(_cylinder(Mesh(), soft=False))
    vp = _stub(_scene_with(hard))
    facets = vp._group_chunk(hard)["vcol"]
    lines = vp._group_chunk(hard)["edges"]
    group = Group(_cylinder(Mesh(), soft=False))
    scene = _scene_with(group)
    vp = _stub(scene)
    hist = History(scene)
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40, smooth=False))
    assert _soft(group.mesh) == SEAMS and _flat(group.mesh) == SEAMS
    chunk = vp._group_chunk(group)
    assert chunk["vcol"] == facets                    # flat-shaded
    assert len(chunk["edges"]) < len(lines)           # the seams' lines gone
    # The disk cache's key tells soft only from soft and smooth.
    from tests.controller_surface import ControllerSurface as Viewport
    soft_only = Viewport._mesh_fingerprint(group.mesh)
    # The same edges with «Smooth normals» on: soft and smooth now.
    hist.execute(SoftenByAngleCommand(selected_edges(scene), 40))
    assert Viewport._mesh_fingerprint(group.mesh) != soft_only
    assert _soft(group.mesh) == SEAMS and _flat(group.mesh) == 0
    assert vp._group_chunk(group)["vcol"] != facets
    hist.undo()
    assert _flat(group.mesh) == SEAMS
    assert vp._group_chunk(group)["vcol"] == facets
    hist.undo()
    assert _soft(group.mesh) == 0


def test_smooth_normals_on_by_default_matches_the_old_soft_flag():
    """A soft edge from before the split (no ``flat``) shades as one the
    command softens with «Smooth normals» on: soft implies smooth."""
    old = Group(_cylinder(Mesh(), soft=True))
    new = Group(_cylinder(Mesh(), soft=False))
    scene = _scene_with(new)
    History(scene).execute(SoftenByAngleCommand(selected_edges(scene), 40))
    assert _stub(_scene_with(old))._group_chunk(old)["vcol"] == \
        _stub(scene)._group_chunk(new)["vcol"]


def test_hardening_clears_the_soft_only_mark():
    group = Group(_cylinder(Mesh(), soft=False))
    scene = _scene_with(group)
    hist = History(scene)
    targets = selected_edges(scene)
    hist.execute(SoftenByAngleCommand(targets, 40, smooth=False))
    hist.execute(SoftenByAngleCommand(targets, harden=True))
    assert _soft(group.mesh) == 0
    assert not any(e.flat for e in group.mesh.edges)


def test_metal_shades_a_soft_only_edge_flat_without_its_line():
    from views.wgpu_scene import pack_scene

    def pack(smooth=None):
        group = Group(_cylinder(Mesh(), soft=False))
        group.xform = QMatrix4x4()
        scene = _scene_with(group)
        if smooth is not None:
            History(scene).execute(SoftenByAngleCommand(
                selected_edges(scene), 40, smooth=smooth))
        return pack_scene(scene)
    def shading(p):
        v = p.vertices
        return {(tuple(np.round(a, 4)), tuple(np.round(n, 4)))
                for a, n in zip(v["pos"], v["normal"])}

    def lines(p):
        return sum(int(c[0]) for c in p.edge_commands)
    hard, flat, smooth = pack(), pack(False), pack(True)
    assert shading(flat) <= shading(hard)              # the facets' normals
    assert not shading(smooth) <= shading(hard)        # the fans' normals
    assert lines(flat) == lines(smooth) < lines(hard)


def test_the_dialog_has_smooth_normals_ticked_as_.skp():
    from ingetrazo_render import OperationForm
    form = OperationForm.soften("30")
    assert form.values()["smooth"]
    form.receive("harden", "1")
    assert all(row[8] & 1 for row in form.rows() if row[0] != "harden")
    form.receive("smooth", "0")
    assert form.values()["smooth"]


def test_the_dialog_passes_smooth_normals_to_the_command(native_window, monkeypatch, tmp_path):
    monkeypatch.setenv("INGETRAZO_AUTOSAVE_DIR", str(tmp_path / "autosave"))
    from ingetrazo_render import chrome_forms
    win = native_window
    forms = []
    monkeypatch.setattr(chrome_forms, "show", lambda _, form: forms.append(form))
    scene = win.viewport.scene
    group = Group(_cylinder(Mesh(), soft=False))
    scene.groups = [group]
    scene.selection = {group}

    win._on_soften_edges()
    form = forms.pop()
    form.receive("angle", "40")
    form.receive("smooth", "0")
    form.press("ok")
    assert _soft(group.mesh) == SEAMS and _flat(group.mesh) == SEAMS


def _half_soft_only_cylinder():
    """A cylinder whose side seams are soft: half of them smooth, half
    soft only (``flat``)."""
    mesh = _cylinder(Mesh(), soft=True)
    seams = [e for e in mesh.edges if e.soft]
    for e in seams[::2]:
        e.flat = True
    return mesh


@pytest.mark.parametrize("binary", [False, True])
@pytest.mark.parametrize("engine", ["python", "native"])
def test_soft_only_edges_survive_save_and_reopen(tmp_path, monkeypatch,
                                                 binary, engine):
    from core import _native
    from formats import igz
    if binary:
        monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
        monkeypatch.setattr(igz, "_BULK_MIN", 1)
    monkeypatch.setattr(_native, "wanted", lambda: engine != "python")
    group = Group(_half_soft_only_cylinder(), name="c")
    group.xform = QMatrix4x4()
    scene = Scene(groups=[group])
    reference = _stub(scene)._group_chunk(group)["vcol"]
    path = tmp_path / "c.igz"
    igz.save_scene(scene, path)
    back = Scene()
    igz.load_into(back, path)
    (g,) = back.groups
    # Drawn before anything builds it, then as built: the same shading.
    assert _stub(back)._group_chunk(g)["vcol"] == reference
    assert _soft(g.mesh) == SEAMS and _flat(g.mesh) == SEAMS // 2
    # Saved again from what was read: still the same flags.
    igz.save_scene(back, path)
    again = Scene()
    igz.load_into(again, path)
    assert _flat(again.groups[0].mesh) == SEAMS // 2


def test_a_file_from_before_the_split_reads_soft_as_smooth(tmp_path):
    """An edge saved before ``flat`` existed — ``"soft": true`` and nothing
    else — is soft and smooth, as it always was."""
    from formats import igz
    group = Group(_cylinder(Mesh(), soft=True), name="c")
    group.xform = QMatrix4x4()
    path = tmp_path / "old.igz"
    igz.save_scene(Scene(groups=[group]), path)
    assert b'"soft"' in path.read_bytes()
    assert b'"flat"' not in path.read_bytes()
    back = Scene()
    igz.load_into(back, path)
    assert _soft(back.groups[0].mesh) == SEAMS
    assert _flat(back.groups[0].mesh) == 0


def test_a_store_backed_edge_keeps_its_soft_only_mark(tmp_path, monkeypatch):
    from core import _native
    from formats import igz
    monkeypatch.setattr(_native, "wanted", lambda: True)
    monkeypatch.setattr(igz, "_LARGE_DOCUMENT", 0)
    monkeypatch.setattr(igz, "_BULK_MIN", 1)
    group = Group(_cylinder(Mesh(), soft=False), name="c")
    group.xform = QMatrix4x4()
    path = tmp_path / "s.igz"
    igz.save_scene(Scene(groups=[group]), path)
    back = Scene()
    igz.load_into(back, path)
    (g,) = back.groups
    back.selection = {g}
    hist = History(back)
    hist.execute(SoftenByAngleCommand(selected_edges(back), 40, smooth=False))
    assert _soft(g.mesh) == SEAMS and _flat(g.mesh) == SEAMS
    hist.undo()
    assert _soft(g.mesh) == 0 and _flat(g.mesh) == 0
    hist.redo()
    assert _flat(g.mesh) == SEAMS
