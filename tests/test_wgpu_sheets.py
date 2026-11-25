"""Sheet dispatch and isolation contracts, independent of a Metal device."""
import os
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
from types import SimpleNamespace
import pytest
from tests.shell_isolation import restore_shell_patches  # noqa: F401
from core.composition import MarcoVista
from core.saved_views import SavedView


def test_offscreen_size_and_live_camera_unchanged(monkeypatch):
    import sys
    import views.wgpu_view as module
    from tests.controller_surface import ControllerSurface as Viewport
    from core.scene import Scene
    from core.camera import OrbitCamera
    class Renderer:
        def set_axes(self, frame, spacing, visible):
            assert len(frame) == 16
        def set_section_geometry(self, data):
            pass
        def render_image(self, w, h, view, projection, payload, rows):
            assert len(view) == len(projection) == 16
            assert 'section_plane' in payload
            return bytes(w*h*4)
    monkeypatch.setattr(module, 'pack_scene', lambda *a: pytest.fail('resident scene rebuilt'))
    source = SimpleNamespace(scene=Scene(), camera=OrbitCamera(), plano_style=None,
                             style_override=None, _horizon_ndc_y=lambda m: 2.)
    source._effective_style = lambda: Viewport._effective_style(source)
    source._view_style = lambda: None    # no view of its own: the scene's
    from tests.headless_gpu import HeadlessView
    view = HeadlessView()
    view.source_viewport = source
    view.renderer = Renderer()
    view._scene_dirty = False
    from core.section import SectionPlane
    from ingetrazo_core import Vec3 as QVector3D
    source.scene.section_planes = [SectionPlane(QVector3D(), QVector3D(1,0,0), active=True)]
    from ingetrazo_core import pack_scene
    view.packed = pack_scene(source.scene)
    before = dict(source.camera.__dict__)
    image = view.render_image(123, 47)
    assert (image.width, image.height) == (123, 47)
    assert view.render_pixels(123, 47) == (123, 47, bytes(image.rgba()))
    assert source.camera.__dict__ == before
    view.close()


def test_scene_recall_updates_metal_layers_camera_style_and_filter(native_host):
    from core.layers import Layer
    from core.style import Style
    from tests.scene_controls import choose
    win = native_host.window
    vp = win.viewport
    scene = vp.scene
    layer = Layer(name="Detail")
    scene.layers.append(layer)
    layer.visible = False
    win.set_active_style(Style(face_mode="wireframe"))
    vp.camera.distance = 42.0
    saved = SavedView.capture("Plan", scene, vp.camera, win.active_style())
    scene.saved_views.append(saved)
    layer.visible = True
    vp.camera.distance = 1.0
    win.set_active_style(Style())
    scene._navigator_allowed = {"earlier-result"}
    vp.active_view_key = "old"
    scene.version += 1
    choose(win, 0)
    assert win._scene_tabs.marked_view() is saved
    assert vp.camera.distance == 42 and not layer.visible
    assert win.active_style().face_mode == "wireframe"
    assert scene._navigator_allowed is None and vp.active_view_key is None
