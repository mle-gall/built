"""Copy/Paste between Built windows (issue #76): the model clipboard
travels through the system clipboard as a small ``.igz``-based payload."""
from __future__ import annotations

import pytest
from built_core import Matrix4 as QMatrix4x4, Vec3 as V

from built_core.group import Group, copy_group
from built_core.mesh import Mesh
from formats import clip_payload as clip
from tests.shell_isolation import restore_shell_patches  # noqa: F401


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


def test_the_clipboard_survives_the_trip():
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


def test_garbage_is_not_a_clipboard():
    assert clip.decode(b"not a zip") is None


@pytest.mark.parametrize("package", ["APPIMAGE", "FLATPAK_ID", None])
def test_new_window_uses_the_running_application(native_host, tmp_path, monkeypatch, package):
    from built_render import Host
    from built_render import wgpu_start

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


def test_without_a_shell_a_copy_reaches_the_other_documents_once(monkeypatch):
    from types import SimpleNamespace
    from built_render import ModelClipboard
    from built_render import shell_services as native

    monkeypatch.setattr(native, "available", lambda: False)
    board = ModelClipboard()
    assert not board.available()
    original = _clip()
    source = SimpleNamespace(viewport=SimpleNamespace(clipboard=original))
    board.publish(source, original)
    assert board.available()
    board.adopt(source)                      # this document's own copy stays
    assert source.viewport.clipboard is original
    released = []
    previous = _clip()
    vp = SimpleNamespace(clipboard=previous, _drop_clip_protos=released.append)
    win = SimpleNamespace(viewport=vp)
    board.adopt(win)
    adopted = vp.clipboard
    assert adopted is not original and adopted["groups"][0].mesh is not original["groups"][0].mesh
    assert vp._clip_adopted_from is original and released == [previous]
    board.adopt(win)
    assert vp.clipboard is adopted and len(released) == 1
