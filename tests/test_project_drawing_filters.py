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
    def __init__(self, *tags, scene=None):
        self.scene = scene if scene is not None else Scene()
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


def test_first_construction_tag_keeps_existing_folder_rows():
    p = Project()
    folder = layers.add_folder(p.scene, "Zone")
    p.scene.layers.extend(Layer(n, folder=folder) for n in ("Walls", "Floors"))
    previous = p.action("toggle", "preview-folder:" + folder)
    p.scene.layers.append(Layer("Guides"))
    p.scene.version += 1
    rows = p.rows()
    assert all(rows[k] == row for k, row in previous.items() if k.startswith("preview-"))
    assert rows["preview-tag:Guides"][0] == "Guides"


def test_a_new_folder_shows_even_empty():
    p = Project()
    p.action("new", "")
    key = next(k for k, (name, _) in p.rows().items() if name == "New node")
    assert p.rows()[key][1][1] == 0
    p.action("rename", key + wgpu_ui.SEP + "Empty folder")
    assert p.rows()[key][0] == "Empty folder"
    assert p.win.viewport.history.undo() and p.rows()[key][0] == "New node"
    assert p.win.viewport.history.undo() and key not in p.rows()


def test_move_the_selection_here_from_the_rows_menu():
    from core import structure
    from tests.test_structure import model
    scene, _, _, loose = model()
    p = Project(scene=scene)
    p.rows()
    p.action("filehere", "n:sections")
    assert not p.win.viewport.history.undo_stack
    scene.select([loose])
    p.action("filehere", "n:sections")
    assert structure.node_of(loose) == "sections" and scene.layer(loose.layer).name == "Track Sections"
    assert p.win.viewport.history.undo() and structure.node_of(loose) is None


def drawing_edit(project, verb, data):
    import json
    project.action(verb, json.dumps(data))


def test_eye_on_a_selected_row_acts_on_every_selected_row():
    project = Project(*(Layer(name) for name in "ABC"))
    scene = project.scene
    drawing_edit(project, "tagstate", {"tags": ["A", "B"], "visible": False})
    assert [scene.layer(n).visible for n in "ABC"] == [False, False, True]
    project.action("lock", "preview-tag:C")
    assert scene.layer("C").locked
    history = project.win.viewport.history
    assert history.undo() and not scene.layer("C").locked
    assert history.undo() and all(scene.layer(n).visible for n in "ABC")


def test_look_menus_set_the_line_and_opacity():
    project = Project(Layer("A"))
    drawing_edit(project, "drawingpatch", [["A"], {"linetype": "dotted", "weight": "0.25", "opacity": "0.5"}])
    tag = project.scene.layer("A")
    assert (tag.linetype, tag.lineweight, tag.opacity) == ("dotted", .25, .5)


def test_several_selected_rows_take_one_colour_and_one_look():
    project = Project(*(Layer(name) for name in "ABC"))
    scene = project.scene
    style = {"color": "#ff0000", "linetype": "dashed", "weight": "0.5", "opacity": "0.5"}
    drawing_edit(project, "drawingpatch", [["A", "B"], style])
    for tag in (scene.layer("A"), scene.layer("B")):
        assert tuple(tag.color) == (1, 0, 0)
        assert (tag.linetype, tag.lineweight, tag.opacity) == ("dashed", .5, .5)
    assert scene.layer("C").color is None
    drawing_edit(project, "tagstate", {"tags": ["A", "B"], "visible": False})
    assert [scene.layer(n).visible for n in "ABC"] == [False, False, True]
    drawing_edit(project, "drawingpatch", [["A", "B"], {"color": ""}])
    assert scene.layer("A").color is None and scene.layer("B").color is None


def test_cut_pen_defaults_and_transparency_apply_to_the_selected_tags():
    project = Project(Layer("A", opacity=.5), Layer("B"), Layer("C"))
    drawing_edit(project, "drawingpatch", [["A", "B"], {"cut_weight": "0.70", "opacity": "0.25"}])
    for name in ("A", "B"):
        layer = project.scene.layer(name)
        assert (layer.cut_weight, layer.lineweight, layer.opacity) == (.7, None, .25)
    assert project.scene.layer("C").cut_weight is None and project.scene.layer("C").opacity == 1
    drawing_edit(project, "drawingpatch", [["A", "B"], {"cut_weight": "auto"}])
    assert all(layer.cut_weight is None for layer in project.scene.layers)
    project.win.viewport.history.undo()
    assert project.scene.layer("B").cut_weight == .7


def test_unprinted_tags_can_be_found_and_bulk_print_preserves_visibility():
    project = Project(*(Layer(name) for name in "ABC"))
    drawing_edit(project, "drawingpatch", [["A", "B"], {"print": "0"}])
    assert project.names("is:unprinted", "preview-tag:") == ["A", "B"]
    assert all(project.scene.layer(name).visible for name in "ABC")
    names = [tag.name for tag in project.scene.layers]
    for flag in (True, False):
        drawing_edit(project, "drawingpatch", [names, {"print": str(int(flag))}])
        assert all(tag.printable == flag for tag in project.scene.layers)
