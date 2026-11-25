# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Camera ▸ Axes: the world's red/green/blue drawing axes can be switched off.
Only the world's: a group open for editing keeps showing its own axes. The
flag travels in the .igz (only when off) and a new or opened document
brings them back. The render checks need a GL context that really draws;
skipped where the platform has none or draws nothing."""
from __future__ import annotations

from tests.controller_surface import live_image
from tests.raster_samples import pixels
from tests.shell_isolation import native_gpu_host, native_window, restore_shell_patches  # noqa: F401
from tests.test_xray_toggle import _state
from views import view_commands
from built_core import Matrix4 as QMatrix4x4, Vec3 as V

from built_core.group import Group  # noqa: E402
from built_core.mesh import Mesh  # noqa: E402
from built_core.scene import Scene  # noqa: E402
from formats import igz  # noqa: E402

def _square(x0, y0=0.0):
    m = Mesh()
    m.add_face([V(x0, y0, 0), V(x0 + 2, y0, 0), V(x0 + 2, y0 + 2, 0),
                V(x0, y0 + 2, 0)])
    return m


# ---- the document ----------------------------------------------------------

def test_axes_are_shown_by_default():
    assert Scene().show_axes is True


def test_hidden_axes_travel_in_the_file(tmp_path):
    s = Scene()
    s.mesh = _square(0)
    s.show_axes = False
    path = tmp_path / "sin-ejes.igz"
    igz.save_scene(s, path)
    assert b"axes_hidden" in path.read_bytes()
    back = Scene()
    igz.load_into(back, path)
    assert back.show_axes is False


def test_shown_axes_stay_implicit_and_older_files_open_with_them(tmp_path):
    s = Scene()
    s.mesh = _square(0)
    path = tmp_path / "con-ejes.igz"
    igz.save_scene(s, path)
    assert b"axes_hidden" not in path.read_bytes()
    back = Scene()
    back.show_axes = False               # left over from a previous document
    igz.load_into(back, path)
    assert back.show_axes is True


def test_a_new_document_brings_the_axes_back():
    s = Scene()
    s.mesh = _square(0)
    s.show_axes = False
    s.clear()
    assert s.show_axes is True


# ---- the menu --------------------------------------------------------------

def test_the_view_menu_switches_the_scene_flag(native_window):
    for expected in (False, True):
        assert view_commands.run(native_window, "axes")
        assert native_window.viewport.scene.show_axes is expected


def test_alt_a_is_its_own_shortcut():
    from built_render import command_registry
    assert [r[0] for r in command_registry([], []) if "Alt+A" in r[3]] == ["view:axes"]


def test_the_checkmark_follows_the_document(native_window):
    scene = native_window.viewport.scene
    for expected in (False, True):
        scene.show_axes = expected
        assert _state(native_window, "axes") == int(expected)
        assert scene.show_axes is expected


# ---- the render ------------------------------------------------------------

def _reddish_near(vp, p, radius=3):
    vp.update()
    img = live_image(vp)
    x, y = vp._world_to_pixel(p)
    rgb = pixels(img)
    r = img.width / vp.width()
    cx, cy = int(x * r), int(y * r)
    for dx in range(-radius, radius + 1):
        for dy in range(-radius, radius + 1):
            red, green, blue = rgb[cy + dy, cx + dx, :3] / 255
            if red > 0.6 and green < 0.45 and blue < 0.5:
                return True
    return False


def test_hiding_drops_the_world_axes_but_an_open_group_keeps_its_own(native_gpu_host):
    w = native_gpu_host.window
    vp = w.viewport
    sc = vp.scene
    sc.groups.clear()
    comp = Group(_square(-6, -1), name="caja")
    comp.xform = QMatrix4x4()
    comp.xform.translate(0, 5, 0)            # its own axes at (0, 5, 0)
    sc.groups.append(comp)
    sc.version += 1
    vp.camera.set_view("top")
    vp.camera.target = V(2, 2.5, 0)
    vp.camera.distance = 20

    on_world_x = V(3, 0, 0)
    assert _reddish_near(vp, on_world_x)
    assert view_commands.run(w, "axes")
    assert not _reddish_near(vp, on_world_x)

    vp.begin_group_edit(comp)
    assert sc.edit_group is comp and sc.drawing_frame is not None
    assert _reddish_near(vp, V(3, 5, 0))     # the group's own red axis
    vp.end_group_edit()
    assert not _reddish_near(vp, on_world_x)  # back out: still hidden

    assert view_commands.run(w, "axes")
    assert _reddish_near(vp, on_world_x)
