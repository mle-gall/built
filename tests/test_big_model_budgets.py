# SPDX-License-Identifier: GPL-3.0-or-later
"""Interaction budgets on a ride-scale model: a 200k-face loose mesh at the
root, another inside a group, nested assemblies, a yard of 3,000 parts and
parts kilometres away (the extent of a site model).

The shell's own Host draws offscreen in this process; each figure is the
best of a few tries, so one hiccup of the machine does not decide it. On a
machine already busy (the full suite on every core) a miss is reported and
skipped, with the figures, rather than failed: the quiet single-worker pass
(``-m perf -p no:xdist``) is the one that holds them.

Commit timings include the tool action, idle processing and the next frame.
The interactive edit targets are 50 ms (120 ms for Push/Pull), with a
20 percent allowance in the budget assertions.
"""
import math
import os
import statistics
import time

import pytest
from built_core import Input as Qt, Matrix4, Vec3 as V

from tests.shell_isolation import attach_test_renderer, open_test_host

pytestmark = pytest.mark.perf

# One table: what each interaction may cost, in milliseconds.
BUDGET_MS = {
    "line_commit_200k": 60,     # release to the next frame, on the 200k mesh
    "erase_commit_200k": 60,
    "move_commit_200k": 60,
    "push_commit_200k": 144,
    "select_hover_200k": 3,      # median of a Select hover over the 200k mesh
    "select_all_view_frame": 100,  # the frame that shows a select-all's highlight
    "select_all_panel_frame": 50,  # worst frame while the panels catch up
}
GRID = 450                       # 450 x 450 quads: 202,500 faces


def _grid(mesh, n, z=0.0, x0=0.0):
    for i in range(n):
        for j in range(n):
            x, y = x0 + i, j
            mesh.add_face([V(x, y, z), V(x + 1, y, z), V(x + 1, y + 1, z), V(x, y + 1, z)])


def _at(x, y, z=0.0):
    m = Matrix4()
    m.translate(x, y, z)
    return m


def _box(mesh, s, h):
    p = [V(0, 0, 0), V(s, 0, 0), V(s, s, 0), V(0, s, 0)]
    q = [V(v.x(), v.y(), h) for v in p]
    mesh.add_face(p[::-1])
    mesh.add_face(q)
    for k in range(4):
        a, b = p[k], p[(k + 1) % 4]
        mesh.add_face([a, b, V(b.x(), b.y(), h), V(a.x(), a.y(), h)])


def big_model(path):
    """The fixture, saved as .igz at ``path``."""
    from built_core.group import Group
    from built_core.mesh import Mesh
    from built_core.scene import Scene
    from formats import igz
    scene = Scene()
    _grid(scene.mesh, GRID)
    parts = []
    for k in range(3):
        m = Mesh()
        _box(m, .4 + .1 * k, .5 + .2 * k)
        parts.append(m)
    for a in range(10):                          # nested assemblies, 3 deep
        top = Group(name=f"Block {a}")
        top.xform = _at(-60 - (a % 5) * 22, (a // 5) * 22)
        for b in range(10):
            mid = Group(name=f"Row {a}.{b}")
            mid.xform = _at(0, b * 2, .3)
            for c in range(10):
                leaf = Group(parts[(a + b + c) % 3], name=f"Part {a}.{b}.{c}")
                leaf.xform = _at(c * 2, 0)
                mid.adopt([leaf])
            top.adopt([mid])
        scene.groups.append(top)
    outer = Group(name="Terrain block")          # a big mesh two deep
    outer.xform = _at(0, -GRID - 20)
    inner = Group(name="Terrain")
    _grid(inner.mesh, GRID, z=-1)
    outer.adopt([inner])
    scene.groups.append(outer)
    yard = Group(name="Yard")                    # thousands of children
    yard.xform = _at(-60, -200)
    for i in range(3000):
        leaf = Group(parts[i % 3], name=f"Crate {i}")
        leaf.xform = _at((i % 60) * 1.5, (i // 60) * 1.5)
        yard.adopt([leaf])
    scene.groups.append(yard)
    for i, (x, y, z) in enumerate([(3000, 0, 0), (0, -2500, 5), (-4000, 1500, 0)]):
        far = Group(parts[i % 3], name=f"Remote {i}")   # kilometres away
        far.xform = _at(x, y, z)
        scene.groups.append(far)
    igz.save_scene(scene, path)
    return path


def _settle(host, rounds=3):
    for _ in range(rounds):
        host.idle()
        host.frame()


def _px(view, p):
    o = getattr(view.packed, "origin", (0, 0, 0))
    got = view._world_to_pixel(V(p.x() - o[0], p.y() - o[1], p.z() - o[2]))
    x, y = (got.x(), got.y()) if hasattr(got, "x") else got[:2]
    return float(x), float(y)


def _key(host, code, mods=0, text=""):
    host.key(True, code, mods, text)
    host.key(False, code, mods, text)


def _busy():
    """Load average past half the cores: other work is running."""
    try:
        return os.getloadavg()[0] > (os.cpu_count() or 1) * .5
    except OSError:
        return False


@pytest.mark.window
@pytest.mark.slow
def test_big_model_interaction_budgets(tmp_path):
    source = big_model(tmp_path / "big.igz")
    ms = {}
    with open_test_host(tmp_path / "profile", width=1200, height=800) as host:
        win = host.window
        assert win.open_path(str(source)) is not False
        _settle(host, 2)
        attach_test_renderer(host)
        _settle(host, 3)
        vp, view = win.viewport, host.view
        deadline = time.monotonic() + 60
        while view.inference_index is None and time.monotonic() < deadline:
            time.sleep(.05)
            _settle(host, 1)
        assert view.inference_index is not None

        def move(p):
            x, y = _px(view, p)
            host.pointer("move", x, y, 0, 0, 0)
            host.idle()

        def click(p):
            x, y = _px(view, p)
            host.pointer("press", x, y, 1, 1, 0)
            host.pointer("release", x, y, 1, 0, 0)

        def timed(action):
            start = time.perf_counter()
            action()
            host.idle()
            host.frame()
            return (time.perf_counter() - start) * 1e3

        # Select's hover over the big mesh.
        host.menu("tool:select")
        vp.camera.set_view("top")
        vp.camera.fit_to(V(0, 0, 0), V(40, 30, 0))
        _settle(host)
        move(V(5.5, 5.5, 0))
        hovers = []
        for i in range(60):
            x, y = _px(view, V(5.0 + .37 * i % 30, 5.03 + .21 * (i % 5), 0))
            start = time.perf_counter()
            host.pointer("move", x, y, 0, 0, 0)
            host.idle()
            hovers.append((time.perf_counter() - start) * 1e3)
        ms["select_hover_200k"] = statistics.median(hovers)

        # Lines and erasures into the 200k-face mesh, each across three cells.
        vp.camera.fit_to(V(0, 0, 0), V(12, 8, 0))
        _settle(host)
        lines, erasures = [], []
        for k in range(3):
            y = 2.5 + k
            host.menu("tool:line")
            _settle(host)
            move(V(3.5, y, 0)); click(V(3.5, y, 0)); _settle(host)
            move(V(6.5, y, 0)); _settle(host)
            faces = len(vp.scene.mesh.faces)
            lines.append(timed(lambda: click(V(6.5, y, 0))))
            assert len(vp.scene.mesh.faces) > faces          # the line cut cells
            _key(host, Qt.Key_Escape.value)
            host.menu("tool:eraser")
            _settle(host)
            move(V(4.5, y, 0)); _settle(host)
            x, yy = _px(view, V(4.5, y, 0))
            host.pointer("press", x, yy, 1, 1, 0)
            _settle(host)
            edges = len(vp.scene.mesh.edges)
            erasures.append(timed(lambda: host.pointer("release", x, yy, 1, 0, 0)))
            assert len(vp.scene.mesh.edges) < edges          # a piece of it went
            _settle(host)
        ms["line_commit_200k"] = min(lines)
        ms["erase_commit_200k"] = min(erasures)

        # Select All: the viewport first, then the panels catch up.
        host.menu("tool:select")
        vp.camera.fit_to(V(-160, -480, -1), V(460, 460, 2))
        _settle(host)
        shown, worst = [], []
        for _ in range(2):
            _key(host, Qt.Key_A.value, Qt.ControlModifier.value, "a")
            assert len(vp.scene.selection) > 400_000
            frames = []
            for _ in range(6):
                start = time.perf_counter()
                host.idle()
                host.frame()
                frames.append((time.perf_counter() - start) * 1e3)
            assert view._panels_stage == 3
            shown.append(frames[0])
            worst.append(max(frames[1:]))
            _key(host, Qt.Key_Escape.value)
            _settle(host, 3)
        ms["select_all_view_frame"] = min(shown)
        ms["select_all_panel_frame"] = min(worst)

        vp.camera.fit_to(V(0, 0, 0), V(12, 8, 0))
        _settle(host)
        moves, pushes = [], []
        for k in range(3):
            p = V(1 + k, 7, 0)
            q = V(p.x() + .3, p.y() + .2, 0)
            host.menu("tool:move")
            move(p)
            click(p)
            _settle(host)
            move(q)
            _settle(host)
            revision = vp.scene.mesh.revision
            moves.append(timed(lambda: click(q)))
            assert vp.scene.mesh.revision > revision
            _key(host, Qt.Key_Escape.value)

        for k in range(3):
            p = V(8.5 + k, 1.5, 0)
            q = V(p.x(), p.y() + .6, 0)
            host.menu("tool:pushpull")
            move(p)
            click(p)
            _settle(host)
            move(q)
            _settle(host)
            faces = len(vp.scene.mesh.faces)
            pushes.append(timed(lambda: click(q)))
            assert len(vp.scene.mesh.faces) > faces
            _key(host, Qt.Key_Escape.value)
        ms["move_commit_200k"] = min(moves)
        ms["push_commit_200k"] = min(pushes)

    table = "\n".join(f"  {k:<24} {ms[k]:8.1f} ms  (budget {v} ms)" for k, v in BUDGET_MS.items())
    print("\nbig model budgets:\n" + table)
    over = {k: round(ms[k], 1) for k, v in BUDGET_MS.items() if ms[k] >= v}
    if over and _busy():
        pytest.skip(f"machine busy (load {os.getloadavg()[0]:.0f}); over budget: {over}\n{table}")
    assert not over, f"over budget: {over}\n{table}"
