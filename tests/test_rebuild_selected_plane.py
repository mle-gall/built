"""Rebuild Faces (planar) on the selected faces (issue #73, @pacaeiro): in
a 3D model, the plane of the selected faces is rebuilt and the rest is left
alone; with nothing selected the tool keeps its whole-flat-drawing rule."""
from __future__ import annotations

import pytest
from built_core import Vec3 as V

from core.mesh import Face
from tests.shell_isolation import restore_shell_patches  # noqa: F401 (autouse)


def _square(mesh, x, y, s, z=0.0):
    return mesh.add_face([V(x, y, z), V(x + s, y, z),
                          V(x + s, y + s, z), V(x, y + s, z)])


def _raised_box(mesh):
    """A 1 m box floating 1 m up: 3D geometry off the ground plane."""
    c = [V(x, y, z) for z in (1, 2) for y in (5, 6) for x in (5, 6)]
    for q in ([0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4],
              [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]):
        mesh.add_face([c[i] for i in q])


def _ground(mesh):
    return [f for f in mesh.faces
            if all(abs(v.position.z()) < 1e-6 for v in f.loop)]


@pytest.fixture
def drawing(tmp_path):
    from built_render import Host
    host = Host(0, 1280, 800, 2.0, settings=str(tmp_path / "settings"))
    sc = host.window.viewport.scene
    _raised_box(sc.mesh)
    a = _square(sc.mesh, 0, 0, 2)
    _square(sc.mesh, 1, 1, 2)            # overlaps the first one
    sc.version += 1
    yield host, sc, a
    host.close()


def test_the_selected_plane_is_rebuilt_in_a_3d_model(drawing):
    host, sc, a = drawing
    box_before = len(sc.mesh.faces) - 2
    sc.select([a])
    host.menu("rebuild_planar")
    ground = _ground(sc.mesh)
    # Two overlapping squares become their three regions.
    assert len(ground) == 3
    assert len(sc.mesh.faces) - len(ground) == box_before   # box untouched
    assert not host.state()["errors"]


def test_faces_on_different_planes_are_refused(drawing):
    host, sc, a = drawing
    top = next(f for f in sc.mesh.faces
               if all(abs(v.position.z() - 2) < 1e-6 for v in f.loop))
    before = len(sc.mesh.faces)
    sc.select([a, top])
    host.menu("rebuild_planar")
    assert len(sc.mesh.faces) == before
    assert "one plane" in host.window.statusBar().currentMessage() or \
        "mismo plano" in host.window.statusBar().currentMessage()
    assert not host.state()["errors"]


def test_nothing_selected_keeps_the_flat_only_rule(drawing):
    host, sc, a = drawing
    before = len(sc.mesh.faces)
    sc.clear_selection()
    host.menu("rebuild_planar")
    assert len(sc.mesh.faces) == before      # 3D model: left alone, as before
    assert not host.state()["errors"]


def test_one_undo_puts_the_plane_back(drawing):
    host, sc, a = drawing
    before = len(_ground(sc.mesh))
    sc.select([a])
    host.menu("rebuild_planar")
    host.menu("undo")
    assert len(_ground(sc.mesh)) == before
    assert all(isinstance(f, Face) for f in sc.mesh.faces)
    assert not host.state()["errors"]
