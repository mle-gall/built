"""Selecting and revealing edges must preserve deferred imported meshes."""
import pytest
from ingetrazo_core import Matrix4 as QMatrix4x4
from core import _native
from core.mesh import Mesh, DeferredMesh
from core.group import Group
from core.scene import Scene
from formats import igz
from tests.controller_surface import live_image
from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401
from tests.test_rendering_surface_regressions import viewport
from tests.test_viewport_smooth_shading import _cylinder


def test_overlay_buffers_do_not_expand_lazy_mesh_or_follow_unrelated_edits(viewport, tmp_path, monkeypatch):
    native=_native.available()
    if native is None or _native.qt_hypot() is None:
        pytest.skip('native array renderer required')
    monkeypatch.setattr(_native,'wanted',lambda:True)
    monkeypatch.setattr(_native,'native',native)
    monkeypatch.setattr(igz,'_LARGE_DOCUMENT',0)
    monkeypatch.setattr(igz,'_BULK_MIN',1)
    source=Scene();g=Group(_cylinder(Mesh()),'tube');g.xform=QMatrix4x4()
    source.groups.append(g)
    path=tmp_path/'tube.igz';igz.save_scene(source,path)
    loaded=Scene();igz.load_into(loaded,path)
    mesh=loaded.groups[0].mesh
    assert type(mesh) is DeferredMesh
    group = loaded.groups.pop()
    vp = viewport
    vp.scene.groups.append(group)
    vp.scene.show_hidden_geometry = True
    vp.scene.select([group])
    vp.scene.version += 1
    display = vp.keys_stand_in
    live_image(vp)
    packed = display.packed
    geometry = packed.vertices
    assert packed.hidden_edge_commands["count"].sum() > 0
    assert packed.edge_commands["count"].sum() > 0
    assert type(mesh) is DeferredMesh
    vp.scene.version += 1
    live_image(vp)
    assert display.packed.vertices is geometry and type(mesh) is DeferredMesh


def test_edit_surroundings_keep_original_instance_batch(viewport):
    vp = viewport
    mesh = _cylinder(Mesh())
    subject = Group(mesh, "open")
    other = Group(mesh, "other")
    other.xform = QMatrix4x4()
    other.xform.translate(3, 0, 0)
    vp.scene.groups.extend([subject, other])
    vp.scene.version += 1
    vp.set_edit_rest_mode("fade")
    live_image(vp)
    display = vp.keys_stand_in
    def row_and_prototype(group):
        row, = display.uid_rows[group.uid]
        return row, int(display.packed.instances["info"][row, 0])

    resident = row_and_prototype(other)
    vp.begin_group_edit(subject)
    live_image(vp)
    assert row_and_prototype(other) == resident
    vp.end_group_edit()
    live_image(vp)
    assert row_and_prototype(other) == resident


def test_context_reuses_assemblies_sharing_an_empty_container_mesh(viewport):
    vp = viewport
    empty = Mesh()
    opened = Group(empty, 'open')
    other = Group(Mesh(), 'large assembly')
    other.children = [Group(empty, 'empty shared container'),
                      Group(_cylinder(Mesh()), 'unrelated tube')]
    vp.scene.groups.extend([opened, other])
    vp.scene.version += 1
    before = vp._placements_epoch()
    vp.scene.edit_group = opened
    vp.scene.mesh = empty
    vp.scene.version += 1
    after = vp._placements_epoch()
    assert id(other) in vp._epoch_kept_tops(before, after, ctx=True)
    # Actual geometry changes must still invalidate the subtree.
    from ingetrazo_core import Vec3 as QVector3D
    empty.add_edge(QVector3D(1, 0, 0), QVector3D(2, 0, 0))
    vp.scene.version += 1
    changed = vp._placements_epoch()
    assert id(other) not in vp._epoch_kept_tops(after, changed, ctx=True)
    vp.scene.edit_group = None


def test_model_info_only_loads_deferred_meshes_when_visible(native_gpu_host, monkeypatch):
    from types import SimpleNamespace
    from unittest.mock import Mock
    from ingetrazo_core import Vec3
    from core import plugin_panels
    from core.mesh import on_mesh_materialized
    from plugins import model_info

    win, view = native_gpu_host.window, native_gpu_host.view
    scene = win.viewport.scene
    view.renderer.ui_enable(None)
    view._ui_attached = True
    win._panel_factories = {"model_info": model_info.ModelInfoPanel}

    def add_deferred(name):
        mesh = Mesh()
        scene.groups.append(Group(mesh, name))
        def build(target):
            target.add_face([Vec3(0, 0, 0), Vec3(1, 0, 0), Vec3(0, 1, 0)])
        assert DeferredMesh.defer(mesh, build, 1)
        return mesh

    meshes = [add_deferred("first"), add_deferred("second")]

    census = Mock(wraps=model_info._collect_stats)
    monkeypatch.setattr(model_info, "_collect_stats", census)
    materialized = []
    token = on_mesh_materialized.set(materialized.append)
    try:
        plugin_panels.sync(view, win)
        scene.version += 1
        plugin_panels.sync(view, win)
        assert not materialized
        census.assert_not_called()

        assert plugin_panels.open_panel(win, "model_info")
        assert {id(m) for m in materialized} == {id(m) for m in meshes}
        binding = next(b for b in view._plugin_bindings.values() if b.key == "model_info")
        binding.invalidate()
        plugin_panels.sync(view, win)
        census.assert_called_once_with(scene)

        scene.version += 1
        plugin_panels.sync(view, win)
        assert census.call_count == 2
        monkeypatch.setattr("ingetrazo_render.shell_services.pasteboard_write", lambda offers: True)
        binding.model.event("copy", "invoke", None)
        assert census.call_count == 2

        other = SimpleNamespace(id="other", title="Other", slot="utility", sections=lambda: [])
        plugin_panels.show_utility(win, "other", other)
        third = add_deferred("third")
        scene.version += 1
        plugin_panels.sync(view, win)
        assert type(third) is DeferredMesh and len(materialized) == 2
        assert census.call_count == 2
        assert plugin_panels.open_panel(win, "model_info")
        assert census.call_count == 3 and materialized[-1] is third
    finally:
        on_mesh_materialized.reset(token)
