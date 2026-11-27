# SPDX-License-Identifier: GPL-3.0-or-later
"""Interaction budgets on a ride-scale model: a 200k-face loose mesh at the
root, another inside a group, nested assemblies, a yard of 3,000 parts and
parts kilometres away (the extent of a site model).

The shell's own Host draws offscreen in this process. Push/Pull uses the
median of three neighbouring pushes. Other edit figures use the best of
three tries so one hiccup of the machine does not decide them. On a
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
    "move_grip_hover": 3,        # median of a Move hover over groups: box grips, handles
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


def painted_model(path, x0=0., n=GRID):
    """An editable, welded import with shared paint rows and nested fittings."""
    from pathlib import Path
    from built_core.group import Group
    from built_core.layers import Layer
    from built_core.materials import Material
    from built_core.scene import Scene
    from formats import igz
    from tests.test_exact_edit import cylinder

    scene = Scene()
    scene.layers.extend([Layer("Panels"), Layer("Concealed", visible=False)])
    mesh = scene.mesh
    _grid(mesh, n, x0=x0)
    texture = Path(__file__).resolve().parents[1] / "resources/textures/checker.png"
    rows = []
    for i in range(24):
        rgb = (0.2 + i / 40, 0.65 - i / 60, 0.4)
        material = Material(f"Panel finish {i:02}", color=rgb)
        if i % 3 == 0:
            material.texture = dict(path=str(texture), sw=2., sh=2.)
        scene.materials[material.name] = material
        row = dict(material.face_attrs(), back=dict(color=rgb[::-1]), layer="Panels")
        if material.texture:
            row["texture"] = dict(material.texture, uvw=[.5, 0., 0., -x0 * .5, 0., .5, 0., .25])
        rows.append(row)
    rows.extend([dict(hidden=True), dict(layer="Concealed")])
    mesh._attr_table.extend(rows)
    for k, face in enumerate(mesh._store.keys("face")):
        i, j = divmod(k, n)
        row = (i + j) % 24
        if i > n // 2 and j % 31 == 0:
            row = 24 + i % 2
        mesh._store.set_face_attr(face, row)
    for edge in mesh._store.keys("edge")[::41]:
        mesh._store.set_edge_flags(edge, 1)
    fittings, _ = cylinder()
    assembly = Group(name="Fittings")
    sections = []
    for row in range(2):
        section = Group(name=f"Fitting row {row}")
        section.xform = _at(x0 + 20, 20 + row * 4, 0)
        parts = []
        for i in range(2):
            part = Group(fittings.mesh, name=f"Sleeve {row}.{i}")
            part.xform = _at(i * 4, 0, 0)
            parts.append(part)
        section.adopt(parts)
        sections.append(section)
    assembly.adopt(sections)
    scene.groups.append(assembly)
    igz.save_scene(scene, path)
    return path


def test_painted_fixture_keeps_shared_curves_after_reopen(tmp_path):
    from built_core.scene import Scene
    from formats import igz

    scene = Scene()
    igz.load_into(scene, painted_model(tmp_path / "painted.built", n=16))
    assert len(scene.materials) == 24
    assert {face.attrs.get("mat") for face in scene.mesh.faces} - {None} == set(scene.materials)
    assert sum(bool(m.texture) for m in scene.materials.values()) == 8
    sections = scene.groups[0].children
    assert [len(group.children) for group in sections] == [2, 2]
    meshes = [child.mesh for group in sections for child in group.children]
    assert all(mesh is meshes[0] for mesh in meshes)
    assert meshes[0]._store.exact_counts() == (48, 24)
    assert "corner_normals" in meshes[0]._store.display_arrays()


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
@pytest.mark.parametrize("model,x0", [(big_model, 0.), (painted_model, 0.), (painted_model, 500000.)],
                         ids=["assembly", "painted", "painted-far"])
def test_big_model_interaction_budgets(tmp_path, model, x0):
    source = (model(tmp_path / "big.igz") if model is big_model else
              model(tmp_path / "painted.built", x0=x0))
    def point(x, y, z=0.):
        return V(x0 + x, y, z)

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
        vp.camera.fit_to(point(0, 0, 0), point(40, 30, 0))
        _settle(host)
        move(point(5.5, 5.5, 0))
        hovers = []
        for i in range(60):
            x, y = _px(view, point(5.0 + .37 * i % 30, 5.03 + .21 * (i % 5), 0))
            start = time.perf_counter()
            host.pointer("move", x, y, 0, 0, 0)
            host.idle()
            hovers.append((time.perf_counter() - start) * 1e3)
        ms["select_hover_200k"] = statistics.median(hovers)

        # Lines and erasures into the 200k-face mesh, each across three cells.
        vp.camera.fit_to(point(0, 0, 0), point(12, 8, 0))
        _settle(host)
        lines, erasures = [], []
        for k in range(3):
            y = 2.5 + k
            host.menu("tool:line")
            _settle(host)
            move(point(3.5, y, 0)); click(point(3.5, y, 0)); _settle(host)
            move(point(6.5, y, 0)); _settle(host)
            faces = len(vp.scene.mesh.faces)
            lines.append(timed(lambda: click(point(6.5, y, 0))))
            assert len(vp.scene.mesh.faces) > faces          # the line cut cells
            _key(host, Qt.Key_Escape.value)
            host.menu("tool:eraser")
            _settle(host)
            move(point(4.5, y, 0)); _settle(host)
            x, yy = _px(view, point(4.5, y, 0))
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
        vp.camera.fit_to(point(-160, -480, -1), point(460, 460, 2))
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

        vp.camera.fit_to(point(0, 0, 0), point(12, 8, 0))
        _settle(host)
        moves, pushes = [], []
        for k in range(3):
            p = point(1 + k, 7, 0)
            q = V(p.x() + .3, p.y() + .2, 0)
            host.menu("tool:move")
            vertex = vp.scene.mesh.vertex_at(p)
            before = vertex.position_tuple()
            move(p)
            click(p)
            _settle(host)
            move(q)
            _settle(host)
            moves.append(timed(lambda: click(q)))
            assert vertex.position_tuple() != before
            _key(host, Qt.Key_Escape.value)

        for k in range(3):
            p = point(8.5 + k, 1.5, 0)
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
        ms["push_commit_200k"] = statistics.median(pushes)

        # Move's hover over the yard of 3,000 parts and the nested blocks:
        # the box grips and rotation handles come from the cached bounds.
        host.menu("tool:move")
        vp.camera.set_view("top")
        vp.camera.fit_to(V(-170, -205, 0), V(35, 45, 2))
        _settle(host)
        tool = host.view.tool
        move(V(-59.7, -199.7, .3))
        _settle(host)
        assert tool._grip_group is not None and tool._box_grips
        grips = []
        for i in range(60):
            p = (V(-59.7 + (i * 1.5) % 60, -199.7 + 1.5 * (i % 7), .3) if i % 2
                 else V(-59.7 - 22 * (i % 4), .3 + 2 * (i % 9), .6))
            x, y = _px(view, p)
            start = time.perf_counter()
            host.pointer("move", x, y, 0, 0, 0)
            host.idle()
            grips.append((time.perf_counter() - start) * 1e3)
        ms["move_grip_hover"] = statistics.median(grips)

    table = "\n".join(f"  {k:<24} {ms[k]:8.1f} ms  (budget {v} ms)" for k, v in BUDGET_MS.items())
    print("\nbig model budgets:\n" + table)
    over = {k: round(ms[k], 1) for k, v in BUDGET_MS.items() if ms[k] >= v}
    if over and _busy():
        pytest.skip(f"machine busy (load {os.getloadavg()[0]:.0f}); over budget: {over}\n{table}")
    assert not over, f"over budget: {over}\n{table}"
