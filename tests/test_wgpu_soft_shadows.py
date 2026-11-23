"""Soft sun shadows (#1281) on the Metal view: the Preferences level reaches
the native payload as GL's penumbra spread, the receiver shader compiles and
softens with it, and a level set from Preferences asks the Metal view of
every project for a frame.

Default pytest runs Qt offscreen and skips the raster check; run this file
with QT_QPA_PLATFORM=cocoa on macOS to execute it (no widget is shown).
"""
import importlib.util
import inspect
import os
import sys

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
import pytest
from PySide6.QtCore import QSettings
from ingetrazo_core import Vec3 as V
from PySide6.QtWidgets import QApplication, QWidget

_app = QApplication.instance() or QApplication([])

from core.scene import Scene  # noqa: E402
from views.document_interaction import SOFT_SHADOW_SPREAD
from views.wgpu_style import style_payload, sun_payload  # noqa: E402

HAS_RENDERER = importlib.util.find_spec("ingetrazo_render") is not None


def _wall_scene():
    """A floor and a wall standing on it, with the noon sun on."""
    scene = Scene()
    m = scene.loose_mesh
    m.add_face([V(-15, -15, 0), V(15, -15, 0), V(15, 15, 0), V(-15, 15, 0)])
    m.add_face([V(-5, 0, 0), V(5, 0, 0), V(5, 0, 6), V(-5, 0, 6)])
    scene.shadows.enabled = True
    scene.shadows.hour = 12
    return scene


@pytest.mark.parametrize("level", [0, 1, 2, 3])
def test_the_payload_carries_the_level_as_gls_spread(level):
    scene = _wall_scene()
    p = sun_payload(scene, soft_shadows=level)
    assert p["shadows"]
    assert p["shadow_soft"] == pytest.approx(SOFT_SHADOW_SPREAD.get(level, 0.0))
    assert style_payload(scene, soft_shadows=level)["shadow_soft"] == p["shadow_soft"]


def test_off_or_nonsense_is_the_crisp_map():
    scene = _wall_scene()
    assert sun_payload(scene)["shadow_soft"] == 0.0
    assert sun_payload(scene, soft_shadows=7)["shadow_soft"] == 0.0
    scene.shadows.enabled = False
    assert sun_payload(scene, soft_shadows=3)["shadow_soft"] == 0.0


def test_the_metal_view_reads_the_level_off_its_viewport():
    """Both the presented frame and the offscreen export pass the source
    viewport's ``_soft_shadows`` (GL softens exports too)."""
    from views.wgpu_view import WgpuView
    for fn in (WgpuView._present_frame, WgpuView.render_image):
        assert "soft_shadows=getattr(" in inspect.getsource(fn), fn.__name__


class _VP:
    edit_rest_mode = "fade"
    _msaa = 4
    _soft_shadows = 0

    def __init__(self):
        self.updates = 0

    def set_edit_rest_mode(self, mode):
        self.edit_rest_mode = mode

    def update(self):
        self.updates += 1


class _Doc:
    def __init__(self, viewport, stand_in):
        self.viewport, self.stand_in = viewport, stand_in


class _Win(QWidget):
    def __init__(self):
        super().__init__()
        self.viewport = _VP()
        self.metal = _VP()                  # its Metal stand-in
        self._documents = [_Doc(self.viewport, self.metal),
                           _Doc(_VP(), None)]   # a GL-only project
        self._rest_actions = {}

    def all_viewports(self):
        return [d.viewport for d in self._documents]


def test_a_new_level_asks_the_metal_view_for_a_frame(tmp_path, monkeypatch):
    import views.preferences_dialog as prefs_mod
    path = tmp_path / "prefs.ini"
    monkeypatch.setattr(prefs_mod, "QSettings",
                        lambda: QSettings(str(path), QSettings.IniFormat))
    win = _Win()
    dlg = prefs_mod.PreferencesDialog(win)
    dlg._soft_shadows.setChecked(True)
    dlg._soft_level.setCurrentIndex(dlg._soft_level.findData(3))
    dlg.accept()
    assert win.viewport._soft_shadows == 3
    assert win.metal.updates >= 1
    # Its next frame's payload carries the new level.
    assert style_payload(_wall_scene(), soft_shadows=win.viewport._soft_shadows)[
        "shadow_soft"] == SOFT_SHADOW_SPREAD[3]


@pytest.fixture
def native():
    if (not HAS_RENDERER or sys.platform != "darwin"
            or os.environ.get("QT_QPA_PLATFORM") != "cocoa"):
        pytest.skip("Metal raster checks require the crate and an unshown Cocoa NSView")
    from PySide6.QtCore import Qt
    from ingetrazo_render import Renderer
    host = QWidget()
    host.setAttribute(Qt.WA_DontShowOnScreen)
    host.resize(256, 256)
    renderer = Renderer(int(host.winId()), 256, 256, 1.)
    renderer.set_antialias(False)
    renderer.set_edges(False)
    yield renderer
    del renderer
    host.close()


def test_a_level_widens_the_penumbra_on_metal(native):
    """The wall's shadow on the floor: at the strongest level many more
    pixels sit between full shade and full sun than with the crisp map,
    and the core of the shadow stays as dark."""
    from ingetrazo_core import Matrix4 as QMatrix4x4
    from views.wgpu_scene import pack_scene
    scene = _wall_scene()
    pack_scene(scene).upload(native)
    view = QMatrix4x4()
    view.lookAt(V(0, 0, 40), V(0, 0, 0), V(0, 1, 0))
    proj = QMatrix4x4()
    proj.ortho(-15, 15, -15, 15, 1, 80)
    native.set_camera(list(view.data()), list(proj.data()))

    def capture(level, shadows=True):
        style = dict(sky=False, edges=False)
        if shadows:
            style.update(sun_payload(scene, soft_shadows=level))
        native.set_style(style)
        native.request_capture()
        native.render(False)
        rgba = np.frombuffer(bytes(native.take_capture()), np.uint8)
        return rgba.reshape(256, 256, 4)[..., :3].astype(int).sum(axis=2)

    sun = capture(0, shadows=False)
    crisp, soft = capture(0), capture(3)
    shade = crisp.min()
    assert shade < sun.max() - 30, "the wall casts a shadow"
    assert abs(int(soft.min()) - int(shade)) <= 12, "the core stays as dark"

    def partial(img):
        lit = sun - 6
        return int(((img > shade + 6) & (img < lit)).sum())

    assert partial(soft) > 2 * max(partial(crisp), 1), (partial(soft), partial(crisp))
