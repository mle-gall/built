# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Inside a group, the rest of the model turns grey («quiero que al entrar a
un grupo o componente lo demás quede gris», 14-10). The fade went towards
the background, and white faces on a light background stayed white: nothing
said you were inside a group. Renders two groups, opens one, reads pixels.

Requires a native wgpu surface for pixel readback."""
from __future__ import annotations

from tests.raster_samples import pixels, solid_raster
from tests.shell_isolation import native_gpu_host, restore_shell_patches  # noqa: F401


from tests.controller_surface import live_image
import pytest
from ingetrazo_core import Matrix4 as QMatrix4x4, Vec3 as V

from core.group import Group
from core.mesh import Mesh
from views.document_interaction import edit_fade_color


def test_the_fade_colour_is_a_grey_away_from_a_light_background():
    light = (0.93, 0.93, 0.93)
    grey = edit_fade_color(light)
    assert all(abs(c - grey[0]) < 1e-9 for c in grey)      # neutral
    assert light[0] - grey[0] > 0.15                         # clearly darker
    dark = (0.15, 0.15, 0.17)
    assert edit_fade_color(dark)[0] > dark[0] + 0.1          # lifted on dark


def _square(x, comp):
    m = Mesh()
    m.add_face([V(x, 0, 0), V(x + 2, 0, 0), V(x + 2, 2, 0), V(x, 2, 0)])
    g = Group(m, name=f"g{x}")
    if comp:
        g.xform = QMatrix4x4()
    return g


@pytest.mark.parametrize("comp", [False, True], ids=["group", "component"])
def test_the_rest_reads_grey_and_the_group_keeps_its_colour(native_gpu_host, comp):
    w = native_gpu_host.window
    native_gpu_host.resize(1200, 800, 1.)
    vp = w.viewport
    sc = vp.scene
    sc.groups.clear()
    a, b = _square(0, comp), _square(4, comp)
    sc.groups.extend([a, b])
    sc.version += 1
    vp.set_edit_rest_mode("fade")
    vp.camera.set_view("top")
    vp.camera.target = V(3, 1, 0)
    vp.camera.distance = 12

    def colour(p):
        vp.update()
        native_gpu_host.idle()
        img = live_image(vp)
        x, y = vp._world_to_pixel(p)
        r = img.width / vp.width()
        return tuple(map(int, pixels(img)[int(y * r), int(x * r), :3]))

    before = colour(V(5, 1, 0))
    vp.begin_group_edit(a)
    after = colour(V(5, 1, 0))
    inside = colour(V(1, 1, 0))
    def lightness(rgb):
        return (max(rgb) + min(rgb)) / 510.0

    # the other group lost a clear step of lightness: grey, not white
    assert lightness(before) - lightness(after) > 0.08
    # the open one keeps its own colour
    assert abs(lightness(inside) - lightness(before)) < 0.03


@pytest.mark.parametrize("mode", ["fade", "hide"])
def test_people_and_images_go_grey_with_the_rest(native_gpu_host, tmp_path, mode):
    """The face-me figures and the reference images are drawn by passes of
    their own: they kept their full colour on a grey model («al editar un
    grupo o componente las imágenes no se ponen en gris», 20-10)."""
    from core.group import make_billboard_group
    from core.image_plane import ImagePlane
    red = solid_raster(16, 16, (220, 20, 20))
    path = str(tmp_path / "red.png")
    red.save(path)
    w = native_gpu_host.window
    native_gpu_host.resize(1200, 800, 1.)
    vp = w.viewport
    sc = vp.scene
    sc.groups.clear()
    a = _square(0, False)
    person = make_billboard_group(path, 2.0, "person", 1.0,
                                  position=V(-3, 0, 0))
    sc.groups.extend([a, person])
    sc.image_planes.append(ImagePlane(path, V(4, 0, 0), V(2, 0, 0),
                                      V(0, 0, 2), layer=None))
    sc.version += 1
    vp.set_edit_rest_mode(mode)
    vp.camera.set_view("front")
    vp.camera.target = V(0.5, 0, 1)
    vp.camera.distance = 14

    def colour(p):
        vp.update()
        native_gpu_host.idle()
        img = live_image(vp)
        x, y = vp._world_to_pixel(p)
        r = img.width / vp.width()
        return tuple(map(int, pixels(img)[int(y * r), int(x * r), :3]))

    spots = (V(-3, 0, 1), V(5, 0, 1))       # the person, the image
    # A figure's texture is loaded on a timer: wait for it to show.
    import time
    for _ in range(40):
        if all(c[0] > 150 and c[1] < 80
               for c in (colour(p) for p in spots)):
            break
        time.sleep(.05)
    before = [colour(p) for p in spots]
    for c in before:
        assert c[0] > 150 and c[1] < 80, c
    vp.begin_group_edit(a)
    for c in (colour(p) for p in spots):
        # neither the full red of before: washed grey, or gone
        assert c[0] - c[1] < 80, c
