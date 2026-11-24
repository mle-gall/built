"""Selecting thousands of groups syncs the Outliner in one selection, not one per row (2026-11-10)."""
import time

from tests.test_project_drawing_filters import Project

from core.group import Group
from core.mesh import Mesh
from core.scene import Scene
from ingetrazo_core import Vec3 as QVector3D



def _scene(n):
    scene = Scene()
    for i in range(n):
        mesh = Mesh()
        mesh.add_face([QVector3D(i, 0, 0), QVector3D(i + 1, 0, 0),
                       QVector3D(i + 1, 1, 0), QVector3D(i, 1, 0)])
        scene.groups.append(Group(mesh, name=f"g{i}"))
    return scene


def test_a_selection_of_thousands_syncs_in_one_go():
    scene = _scene(3000)
    project = Project(scene=scene)
    project.view._ui_pages = {"": 3000}
    project.action("toggle", "n:")
    scene.select(scene.groups)
    start = time.perf_counter()
    marked = {key for key, (_, flags) in project.rows().items() if flags[10] == 1}
    assert time.perf_counter() - start < 3.0
    assert marked == {"i:" + group.uid for group in scene.groups}
    scene.clear_selection()
    assert not any(flags[10] for _, flags in project.rows().values())
