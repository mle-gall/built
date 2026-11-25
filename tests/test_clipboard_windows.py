"""Copy/Paste between IngeTrazo windows (issue #76): the model clipboard
travels through the system clipboard as a small ``.igz``-based payload."""
from __future__ import annotations

from types import SimpleNamespace

import pytest
import ingetrazo_core as native_clip
from ingetrazo_core import ClipOffer, Matrix4 as QMatrix4x4, Vec3 as V

from core.group import Group, copy_group
from core.mesh import Mesh
from formats.clip_payload import decode, encode
from tests.shell_isolation import restore_shell_patches  # noqa: F401

MIME = "application/x-ingetrazo-clipboard"


@pytest.fixture
def app(monkeypatch):
    """No shell: the offer lives in this process, as in a headless run."""
    from app_shell import native
    monkeypatch.setattr(native, "available", lambda: False)


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
    out = decode(encode(_clip()))
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
    assert decode(b"not a zip") is None


def test_this_windows_own_copy_is_not_foreign(app):
    copied = _clip()
    native_clip.clip_publish(copied)
    assert native_clip.clip_available()
    assert native_clip.clip_foreign() is None
    # Another window of this process pastes its own decoded copy.
    other = native_clip.clip_own({})
    assert other is not None and other is not copied
    assert native_clip.clip_own(copied) is None


def test_the_offer_is_rendered_on_demand(app):
    offer = ClipOffer(_clip())
    assert offer.formats() == [MIME]
    data = offer.retrieveData(MIME, None)
    assert decode(data.data()) is not None
    assert offer.retrieveData("text/plain", None) is None


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
    native_clip.clip_publish(_clip())
    native_clip.clip_flush()
    offer = native_clip.clip_current_offer()
    assert decode(offer.data(MIME).data()) is not None


def test_a_shell_pasteboard_carries_the_copy_to_another_process(monkeypatch):
    from app_shell import native
    board = {"types": [], "count": 0, "data": {}}

    def write(items):
        board["data"] = dict(items)
        board["types"] = list(board["data"])
        board["count"] += 1
        return True

    monkeypatch.setattr(native, "available", lambda: True)
    monkeypatch.setattr(native, "pasteboard_write", write)
    monkeypatch.setattr(native, "pasteboard_types", lambda: (board["types"], board["count"]))
    monkeypatch.setattr(native, "pasteboard_read", lambda kind: board["data"].get(kind))
    native_clip.clip_publish(_clip())
    assert native_clip.clip_available() and native_clip.clip_foreign() is None
    # Another process copies: the offer is no longer ours, and pastes.
    write([(MIME, encode(_clip()))])
    pasted = native_clip.clip_foreign()
    assert pasted is not None and len(pasted["groups"]) == 2


def test_native_window_adopts_a_platform_offer_once(app, monkeypatch):
    import formats.clip as adapter
    from ingetrazo_render import ModelClipboard

    original = _clip()
    offer = ClipOffer(original)
    monkeypatch.setattr(adapter, "current_offer", lambda: offer)
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
