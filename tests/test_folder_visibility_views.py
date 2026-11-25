"""A hidden tag folder hides its tags in sheets/exports, and scenes
remember which folders were switched off."""
import built_core
from built_core import Vec3 as QVector3D

from built_core import layers as L
from built_core.layers import Layer
from built_core.scene import Scene
from built_core.saved_views import SavedView
from built_core.camera import OrbitCamera


def _scene():
    scene = Scene()
    f = built_core.tag_edits.add_folder(scene, "Arq")
    scene.layers += [Layer("Muro", folder=f), Layer("Losa", folder=f,
                                                    visible=False)]
    return scene, f


def test_visible_placements_follow_the_folder():
    from built_core.group import Group
    from built_core import group_visible_placements as visible_placements
    from built_core.mesh import Mesh
    scene, f = _scene()
    child = Group(Mesh(), name="child")
    child.layer = "Muro"
    parent = Group(Mesh(), name="parent")
    parent.children = [child]
    scene.groups.append(parent)
    shown = lambda: [g for g, *_ in visible_placements(scene, parent)]
    assert child in shown()
    scene.folder_visibility[f] = False
    assert child not in shown()


def test_a_scene_remembers_and_restores_hidden_folders():
    scene, f = _scene()
    cam = OrbitCamera()
    scene.folder_visibility[f] = False
    view = SavedView.capture("Plan", scene, cam)
    again = SavedView.from_dict(view.to_dict())
    assert again.hidden_folders == [f]
    scene.folder_visibility[f] = True
    scene.layer("Losa").visible = True
    again.apply_state(scene)
    assert scene.folder_visibility[f] is False
    assert scene.layer("Losa").visible is False       # its own switch
    assert scene.layer("Muro").visible is True        # kept, folder hides it
    assert not built_core.tag_edits.effective_visible(scene, scene.layer("Muro"))


def test_an_old_scene_leaves_the_folders_alone():
    scene, f = _scene()
    old = SavedView.from_dict({"name": "Old", "hidden_layers": ["Losa"]})
    assert old.hidden_folders is None
    scene.folder_visibility[f] = False
    old.apply_state(scene)
    assert scene.folder_visibility[f] is False
