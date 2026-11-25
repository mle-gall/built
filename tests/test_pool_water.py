# SPDX-License-Identifier: GPL-3.0-or-later
"""Pool water is a native translucent paint in both material pickers."""
from __future__ import annotations

from pathlib import Path

from tests.shell_isolation import restore_shell_patches  # noqa: F401
from ingetrazo_core import ScreenPoint as QPointF, Input as Qt
from ingetrazo_core import Vec3 as QVector3D

from core.i18n import tr
from core.scene import Scene
from formats.igz import load_into, save_scene
from tools.base import ToolContext
from tools.paint import PaintTool
from ingetrazo_render.wgpu_library import _catalogue
from views.wgpu_ui import _paint_with


def test_water_library_keeps_calm_water_and_adds_one_static_pool_material():
    entries = [e for e in _catalogue().values() if e["key"].startswith("lib:water:")]
    assert [e["name"] for e in entries] == ["water_calm", tr("Pool Water")]
    calm, pool = (e["recipe"] for e in entries)
    assert calm["texture"]["sw"] == calm["texture"]["sh"] == 2.0
    assert calm["opacity"] == 0.75
    assert pool["texture"]["sw"] == pool["texture"]["sh"] == 3.0
    assert pool["opacity"] == 0.62
    assert Path(pool["texture"]["path"]).is_file()
    from ingetrazo_render import swatch_pixels
    assert len(swatch_pixels(pool["texture"]["path"], 32, True)) == 32 * 32 * 4


def test_pool_swatch_paints_undoes_and_embeds_its_texture(tmp_path, monkeypatch, native_host):
    win, vp = native_host.window, native_host.viewport
    paint_state = {key: value for key, value in vars(PaintTool).items()
                   if key.startswith("current_") or key == "_library_pick"}
    try:
        face = vp.scene.mesh.add_face([QVector3D(0, 0, 0), QVector3D(4, 0, 0),
                                       QVector3D(4, 4, 0), QVector3D(0, 4, 0)])
        before = (vp.scene.version, len(vp.history.undo_stack), dict(vp.scene.materials))
        _paint_with(win, "lib:water:1")
        assert before == (vp.scene.version, len(vp.history.undo_stack), dict(vp.scene.materials))
        assert isinstance(vp.active_tool, PaintTool)
        monkeypatch.setattr(vp, "pick_face_any", lambda *_: (face, None))
        vp.active_tool.on_click(ToolContext(
            viewport=vp, world=QVector3D(2, 2, 0), screen=QPointF(100, 100),
            modifiers=Qt.NoModifier, snap=None))
        assert face.attrs["mat"] == tr("Pool Water")
        assert face.attrs["opacity"] == 0.62
        assert len(vp.history.undo_stack) == before[1] + 1
        vp.history.undo()
        assert not face.attrs
        vp.history.redo()
        path = tmp_path / "pool.igz"
        save_scene(vp.scene, path)
        loaded = Scene()
        load_into(loaded, path)
        pool = loaded.materials[tr("Pool Water")]
        assert pool.opacity == 0.62
        assert pool.texture["sw"] == pool.texture["sh"] == 3.0
        embedded = Path(pool.texture["path"])
        assert embedded.is_file()
        assert embedded.read_bytes() == Path(PaintTool.current_texture["path"]).read_bytes()
    finally:
        for key, value in paint_state.items():
            setattr(PaintTool, key, value)
