# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
"""Back Edges (K, issue #234, @ales-limon): the edges a face hides are drawn
dashed over the opaque model — where a bar continues behind a face in a
shop drawing. Renders a box in front of a horizontal line and a diagonal
one and reads the pixels.

Pixel cases use the native wgpu surface."""
from __future__ import annotations

from tests.controller_surface import live_image
from tests.shell_isolation import native_gpu_host, native_window, restore_shell_patches  # noqa: F401
from tests.test_xray_toggle import _state
from views import view_commands
import pytest
from ingetrazo_core import Vec3 as V


def _box(scene, x0, y0, z0, x1, y1, z1):
    p = [V(x0, y0, z0), V(x1, y0, z0), V(x1, y1, z0), V(x0, y1, z0),
         V(x0, y0, z1), V(x1, y0, z1), V(x1, y1, z1), V(x0, y1, z1)]
    for idx in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5),
                (2, 3, 7, 6), (3, 0, 4, 7)):
        scene.mesh.add_face([p[i] for i in idx])


def _dark(c, ref=None):
    """Ink on paper: clearly darker than the white faces and the grey
    background (antialiasing spreads a 1 px line over two rows)."""
    return c.lightnessF() < 0.72


@pytest.fixture
def win(native_gpu_host):
    w = native_gpu_host.window
    vp = w.viewport
    sc = vp.scene
    sc.guides.clear()
    _box(sc, -1, -1, 0, 1, 0, 2)                          # the wall, in front
    sc.mesh.add_edge(V(-3, 1, 1.0), V(3, 1, 1.0))         # a bar behind it
    sc.mesh.add_edge(V(-0.8, 1, 0.2), V(0.8, 1, 1.8))     # and a diagonal one
    sc.version += 1
    st = sc.display_style
    st.face_mode, st.sky, st.profiles = "hidden_line", False, False
    vp.camera.set_view("front")                           # looking toward +Y
    vp.camera.perspective = False
    vp.camera.fit_to(V(-3.2, -1, 0), V(3.2, 1, 2))
    return w


def _row(win, pts):
    """The darkest pixel ACROSS the line at each point (±2 px along the
    screen normal of the run of ``pts``): antialiasing spreads a 1 px line
    over two rows, and searching along it would bridge the gaps."""
    import math
    vp = win.viewport
    img = live_image(vp)
    dpr = img.width() / max(vp.width(), 1)
    a, b = vp._world_to_pixel(pts[0]), vp._world_to_pixel(pts[-1])
    dx, dy = b[0] - a[0], b[1] - a[1]
    ln = math.hypot(dx, dy) or 1.0
    nx, ny = -dy / ln, dx / ln
    out = []
    for p in pts:
        px = vp._world_to_pixel(p)
        x, y = px[0] * dpr, px[1] * dpr
        # floor, not round: round() takes halves to even, and a line at
        # y = 339.5 was sampled at 338, 338, 340, 340, 342 — never 339.
        out.append(min((img.pixelColor(math.floor(x + k * nx), math.floor(y + k * ny))
                        for k in (-2, -1, 0, 1, 2)),
                       key=lambda c: c.lightnessF()))
    return out


def _samples(y=1.0, z=1.0, x0=-0.7, x1=0.7, n=60):
    return [V(x0 + (x1 - x0) * i / (n - 1), y, z) for i in range(n)]


def test_hidden_edges_stay_hidden_by_default(win):
    ec = win.viewport.scene.display_style.edge_color
    assert not win.viewport.scene.display_style.back_edges
    behind = _row(win, _samples())
    assert not any(_dark(c, ec) for c in behind)
    # the part of the bar that no face hides is drawn solid
    outside = _row(win, _samples(x0=1.6, x1=2.8))
    assert all(_dark(c, ec) for c in outside)


def test_back_edges_draws_them_dashed(win):
    st = win.viewport.scene.display_style
    ec = st.edge_color
    assert view_commands.run(win, "back_edges")
    assert st.back_edges
    behind = [_dark(c, ec) for c in _row(win, _samples())]
    assert 0.25 < sum(behind) / len(behind) < 0.75, behind   # dashes, not solid
    runs = sum(1 for a, b in zip(behind, behind[1:]) if a != b)
    assert runs >= 6
    # the diagonal too: a screen-space pattern would be all dash or all gap
    diag = [_dark(c, ec) for c in _row(
        win, [V(-0.6 + 1.2 * i / 59, 1, 0.4 + 1.2 * i / 59) for i in range(60)])]
    # (without antialiasing a 1 px diagonal is a staircase that sampling
    # across it partly misses: neither solid nor empty is what matters)
    assert 0.12 < sum(diag) / len(diag) < 0.75, diag
    assert sum(1 for a, b in zip(diag, diag[1:]) if a != b) >= 6
    # the visible part of the bar is still solid
    assert all(_dark(c, ec) for c in _row(win, _samples(x0=1.6, x1=2.8)))


def test_k_is_back_edges_and_the_style_remembers_it(native_window):
    from core.style import Style
    s = Style(back_edges=True)
    assert Style.from_dict(s.to_dict()).back_edges is True
    assert Style.from_dict({"name": "Old"}).back_edges is False
    from ingetrazo_render import command_registry
    assert [r[0] for r in command_registry([], []) if "K" in r[3]] == ["view:back_edges"]
    w = native_window
    before = w.active_style().back_edges
    assert view_commands.run(w, "back_edges") and _state(w, "back_edges") == int(not before)
    assert view_commands.run(w, "back_edges") and w.active_style().back_edges == before
    w._view_bindings = {"back_edges": ("K", False)}
    assert not view_commands.run(w, "back_edges") and w.active_style().back_edges == before
