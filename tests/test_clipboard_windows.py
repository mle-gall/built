"""Copy/Paste between IngeTrazo windows (issue #76): the model clipboard
travels through the system clipboard as a small ``.igz``-based payload."""
from __future__ import annotations

import pytest
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V
from PySide6.QtWidgets import QApplication

from core.group import Group, copy_group
from core.mesh import Mesh
from formats import clip
from tests.shell_isolation import restore_shell_patches  # noqa: F401


@pytest.fixture(scope="module")
def app():
    return QApplication.instance() or QApplication([])


def _box_group(x: float) -> Group:
    m = Mesh()
    pts = [V(x, 0, 0), V(x + 1, 0, 0), V(x + 1, 1, 0), V(x, 1, 0)]
    m.add_face(pts)
    g = Group(m)
    g.name = f"box {x}"
    return g


def _clip():
    square = [V(0, 0, 0), V(2, 0, 0), V(2, 2, 0), V(0, 2, 0)]
    hole = [V(0.5, 0.5, 0), V(1, 0.5, 0), V(1, 1, 0), V(0.5, 1, 0)]
    classic = _box_group(5)
    placed = copy_group(_box_group(9))
    xf = QMatrix4x4()
    xf.translate(0, 0, 3)
    placed.xform = xf
    return {
        "faces": [(square, [hole], {"color": (0.8, 0.2, 0.1)})],
        "edges": [(V(0, 0, 0), V(0, 0, 4), True, None)],
        "groups": [classic, placed],
        "ref": V(0, 0, 0),
    }


def test_the_clipboard_survives_the_trip(app):
    out = clip.decode(clip.encode(_clip()))
    assert out is not None
    (loop, holes, attrs), = out["faces"]
    assert len(loop) == 4 and len(holes) == 1
    assert attrs["color"] == pytest.approx((0.8, 0.2, 0.1))
    (a, b, soft, curve), = out["edges"]
    assert (a, b, soft, curve) == (V(0, 0, 0), V(0, 0, 4), True, None)
    assert out["ref"] == V(0, 0, 0)
    assert len(out["groups"]) == 2
    # Every group arrives as a placement (the paste tool stamps siblings).
    assert all(g.xform is not None for g in out["groups"])
    lifted = out["groups"][1]
    assert lifted.xform.map(V(0, 0, 0)).z() == pytest.approx(3)


def test_garbage_is_not_a_clipboard(app):
    assert clip.decode(b"not a zip") is None


def test_this_windows_own_copy_is_not_foreign(app):
    clip.publish(_clip())
    assert clip.available()
    assert clip.foreign() is None


def test_the_offer_is_rendered_on_demand(app):
    mime = clip.ClipMime(_clip())
    assert mime.formats() == [clip.MIME]
    data = mime.retrieveData(clip.MIME, None)
    assert clip.decode(data.data()) is not None
    assert mime.retrieveData("text/plain", None) is None


@pytest.mark.parametrize("package", ["APPIMAGE", "FLATPAK_ID", None])
def test_new_window_uses_the_running_application(native_host, tmp_path, monkeypatch, package):
    from app_shell.host import Host
    from ingetrazo_render import wgpu_start

    if package:
        monkeypatch.setenv(package, str(tmp_path / "package"))
    native_host.window.run_command("new_window")
    token = native_host.extras()["new_window"]
    assert token
    other = Host(0, 900, 600, 1., settings=str(native_host.settings_dir))
    try:
        assert other.adopt(token)
        assert wgpu_start.shown(other.window)
        assert other.window.viewport.scene is not native_host.window.viewport.scene
        assert native_host.extras()["new_window"] is None
    finally:
        other.close()


def test_quitting_leaves_the_copy_as_plain_data(app):
    clip.publish(_clip())
    clip.flush()
    md = clip.current_offer()
    assert not isinstance(md, clip.ClipMime)
    assert clip.decode(md.data(clip.MIME).data()) is not None


def test_native_window_adopts_a_platform_offer_once(app, monkeypatch):
    from types import SimpleNamespace
    from ingetrazo_render import ModelClipboard
    from app_shell import native

    original = _clip()
    offer = clip.ClipMime(original)
    monkeypatch.setattr(native, "available", lambda: False)
    monkeypatch.setattr(clip, "current_offer", lambda: offer)
    released = []
    previous = _clip()
    vp = SimpleNamespace(clipboard=previous, _drop_clip_protos=released.append)
    win = SimpleNamespace(viewport=vp)
    board = ModelClipboard()
    board.adopt(win)
    adopted = vp.clipboard
    assert adopted is not original and adopted["groups"][0].mesh is not original["groups"][0].mesh
    assert vp._clip_adopted_from is original and released == [previous]
    board.adopt(win)
    assert vp.clipboard is adopted and len(released) == 1
