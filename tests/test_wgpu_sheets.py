"""Sheet dispatch and isolation contracts, independent of a Metal device."""
import os
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
from types import SimpleNamespace
import pytest
from PySide6.QtWidgets import QApplication
from PySide6.QtGui import QImage
from core.composition import MarcoVista
from core.saved_views import SavedView
from tests.test_composer_perspective import _composer

_app = QApplication.instance() or QApplication([])


def test_sheet_dispatch_and_override_restoration():
    composer, host = _composer()
    calls = []
    def render(w, h, **kwargs):
        calls.append((w, h))
        return QImage(w, h, QImage.Format_RGB32)
    host.viewport.render_image = render
    host.viewport.plano_style = 'lineas'
    frame = MarcoVista(w_mm=20, h_mm=10, style='tecnico')
    image = composer.render_frame(frame)
    assert (image.width(), image.height()) == calls[0] == frame.render_px(300)
    assert host.viewport.plano_style == 'lineas'
    composer.close()
    host.close()


def test_saved_frame_restores_hidden_camera_and_filter_on_error():
    from core.group import Group
    from core.mesh import Mesh
    composer, host = _composer()
    vp = host.viewport
    group = Group(Mesh())
    vp.scene.groups.append(group)
    saved = SavedView.capture('sheet', vp.scene, vp.camera)
    saved.hidden_objects = [group.uid]
    saved.two_point = True
    vp.scene.saved_views.append(saved)
    vp.scene._navigator_allowed = {'live'}
    vp.view_tint = {'live': (1., 0., 0.)}
    def fail():
        assert group.hidden
        assert vp.scene._navigator_allowed is None
        raise RuntimeError('capture failed')
    with pytest.raises(RuntimeError):
        composer._with_frame_camera(MarcoVista(view_key='scene:sheet'), fail)
    assert not group.hidden
    assert not vp.camera.two_point
    assert vp.scene._navigator_allowed == {'live'}
    assert vp.view_tint == {'live': (1., 0., 0.)}
    composer.close()
    host.close()


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
    view = module.WgpuView()
    view.source_viewport = source
    view.renderer = Renderer()
    view._scene_dirty = False
    from core.section import SectionPlane
    from ingetrazo_core import Vec3 as QVector3D
    source.scene.section_planes = [SectionPlane(QVector3D(), QVector3D(1,0,0), active=True)]
    from views.wgpu_scene import pack_scene
    view.packed = pack_scene(source.scene)
    before = dict(source.camera.__dict__)
    image = view.render_image(123, 47)
    assert (image.width(), image.height()) == (123, 47)
    assert source.camera.__dict__ == before
    view.close()


def test_scene_recall_updates_metal_layers_camera_style_and_filter():
    from core.layers import Layer
    from core.scene import Scene
    from core.camera import OrbitCamera
    from core.style import Style
    from views.tray import ScenesPanel
    scene, camera = Scene(), OrbitCamera()
    layer = Layer(name='Detail')
    scene.layers.append(layer)
    layer.visible = False
    scene.display_style = Style(face_mode='wireframe')
    camera.distance = 42.
    view = SavedView.capture('Plan', scene, camera)
    layer.visible = True
    camera.distance = 1.
    scene.display_style = Style()
    scene._navigator_allowed = {'old'}
    calls = []
    viewport = SimpleNamespace(scene=scene, camera=camera, update=lambda: None,
                               view_tint=None, active_view_key='old',
                               setFocus=lambda *a: None)
    display = SimpleNamespace(set_scene=lambda s: calls.append(s), update=lambda: None)
    window = SimpleNamespace(viewport=viewport,
                             _doc=SimpleNamespace(_renderer_wgpu=True, stand_in=display),
                             statusBar=lambda: SimpleNamespace(showMessage=lambda *a: None))
    panel = SimpleNamespace(_window=window, _scene=lambda: scene, _touch=lambda: None,
                            _begin_own=lambda: None, refresh=lambda: None,
                            row_for=lambda v: v, shown=None)
    # The panel's own path from a double-click to the recall (no glide:
    # the stand-in window has no scene player).
    for name in ("show_view", "finish_view", "activate_view", "_show"):
        setattr(panel, name, getattr(ScenesPanel, name).__get__(panel))
    ScenesPanel._on_activate(panel, SimpleNamespace(
        data=lambda _column, _role: ("scene", view)))
    assert calls == [scene]
    assert camera.distance == 42.
    assert not layer.visible
    assert scene.display_style.face_mode == 'wireframe'
    assert scene._navigator_allowed is None


def test_saved_navigator_filter_is_temporary():
    from core import bim_registry
    from core.group import Group
    from core.mesh import Mesh
    from views.composer_scene import temporary_display
    composer, host = _composer()
    scene = host.viewport.scene
    a, b = Group(Mesh(), name='A'), Group(Mesh(), name='B')
    scene.groups.extend([a, b])
    registry = bim_registry.get(scene)
    scene.plugin_data['bim_registry'] = registry
    registry['views']['only-a'] = {
        'filter': {'facet': 'name', 'in': ['A']}, 'isolate': True}
    view = SavedView('A', navigator_view='only-a')
    scene.selection.add(b)
    before = scene.version
    with temporary_display(host.viewport, view):
        assert scene._navigator_allowed == {a.uid}
        assert b in scene.selection
    assert scene._navigator_allowed is None
    assert scene.version == before
    composer.close()
    host.close()


def test_sheet_text_font_uses_composer_history(monkeypatch):
    from core.composition import TextoItem
    from views import font_picker
    from views.wgpu_sheets import Workspace
    composer, host = _composer()
    try:
        model = TextoItem(text="Floor plan")
        composer.comp.texts.append(model)
        composer._rebuild_canvas()
        item = next(i for i in composer.canvas.items() if getattr(i, "model", None) is model)
        item.setSelected(True)
        workspace = Workspace(composer)
        workspace.view = SimpleNamespace(window=lambda: host, update=lambda: None)
        index = next(i for i, row in enumerate(workspace.TEXT) if row[0] == "font")

        def choose(win, title, sample, family, bold, italic, size, unit, apply):
            assert sample == model.text and unit.strip() == "pt"
            apply(dict(font="Georgia", bold=True, italic=True, height=20.))
            return True

        monkeypatch.setattr(font_picker, "show", choose)
        workspace.apply(str(index), "")
        assert (model.family, model.bold, model.italic, model.size_pt) == ("Georgia", True, True, 20.)
        assert composer.text_size.value() == 20.
        composer.history.undo()
        assert model.family == "Sans Serif"
    finally:
        composer.close()
        host.close()
