# SPDX-License-Identifier: GPL-3.0-or-later
"""Drawing rows of an imported file in the Project tree before bootstrap."""
import pytest

from core import layers
from core.group import Group
from core.layers import Layer
from core.scene import Scene
from tests.test_tree_links import _View, _Win
from views import wgpu_ui


class Project:
    def __init__(self, *tags):
        self.scene = Scene()
        self.scene.layers.extend(tags)
        self.win = _Win(self.scene)
        self.view = _View()

    def rows(self):
        wgpu_ui._tree(self.view, self.win)
        rows = self.view.renderer.calls["ui_set_tree"][0]
        return {text[0]: (text[1], state) for text, state in rows}

    def action(self, verb, key):
        wgpu_ui._tree_action(self.win, self.view, wgpu_ui.SEP.join((verb, key)))
        return self.rows()

    def names(self, query, prefix="preview-"):
        rows = self.action("filter", query)
        return sorted(name for key, (name, _) in rows.items() if key.startswith(prefix))


@pytest.mark.parametrize("text,expect", [
    ("a-*", ["A-MUROS", "A-PUERTAS"]),
    ("*mur*", ["A-MUROS", "E-MUROS"]),
    ("?-muros", ["A-MUROS", "E-MUROS"]),
    ("puert", ["A-PUERTAS"]),
    ("a-", ["A-MUROS", "A-PUERTAS"]),
    ("?tage", ["Étage"]),
    ("a-?", []),
])
def test_the_search_takes_wildcards(text, expect):
    project = Project(*(Layer(name) for name in ["A-MUROS", "A-PUERTAS", "E-MUROS", "Étage"]))
    assert project.names(text) == expect


def test_filter_hides_what_does_not_match():
    project = Project(Layer("Muros"), Layer("Puertas", folder="Arq"))
    assert project.names("puer") == ["Arq", "Puertas"]
    assert "Muros" in project.names("")


def test_filters_keep_the_folders_on_the_way():
    project = Project(Layer("Oculta", folder="Arq", visible=False), Layer("Vista", folder="Arq"))
    layers.add_folder(project.scene, "Vacia")
    assert project.names("is:hidden") == ["Arq", "Oculta"]


def test_a_folder_eye_reaches_layers_in_folders_inside_it():
    project = Project()
    scene = project.scene
    outer = layers.add_folder(scene, "Arq")
    inner = layers.add_folder(scene, "Muros", parent=outer)
    scene.layers.extend([Layer("Exterior", folder=inner), Layer("Suelo", folder=outer)])
    project.rows()
    project.action("eye", "preview-folder:" + outer)
    for name in ["Exterior", "Suelo"]:
        assert scene.layer(name).visible and not layers.effective_visible(scene, scene.layer(name))
    assert project.names("exter") == sorted([outer, inner, "Exterior"])
    assert project.win.viewport.history.undo()
    assert layers.effective_visible(scene, scene.layer("Exterior"))


def test_successive_eye_actions_toggle_only_the_addressed_tag():
    project = Project(Layer("A"), Layer("B"))
    project.rows()
    for expected in [False, True]:
        rows = project.action("eye", "preview-tag:A")
        assert rows["preview-tag:A"][1][5] == expected
        assert project.scene.layer("A").visible == expected
        assert project.scene.layer("B").visible


def test_the_filter_follows_a_rename():
    project = Project()
    tree = Group(name="Tree")
    project.scene.groups.append(tree)
    assert "i:" + tree.uid not in project.action("filter", "oak")
    project.action("rename", "i:" + tree.uid + wgpu_ui.SEP + "Oak")
    assert project.rows()["i:" + tree.uid][0] == "Oak"


def test_the_filter_finds_planes_by_name():
    from tests.test_project_occurrences import _two_planes
    project = Project()
    a, b = _two_planes(project.scene)
    match = project.action("filter", "Cr?ss")
    assert "section:" + str(b.uid) in match and "section:" + str(a.uid) not in match
    assert all(not key.startswith("section:") for key in project.action("filter", "slab"))
