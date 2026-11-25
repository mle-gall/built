"""Filing, recipe editing, and bootstrap through the Project tree and protocol."""
import pytest

from core import bim_registry, structure
from ingetrazo_core import facets
from core.agent_facets import set_display
from core.group import Group
from core.layers import Layer
from core.protocol import commands
from tests.test_project_drawing_filters import Project
from tests.test_structure import model
from views import wgpu_ui


@pytest.fixture
def panel():
    scene, registry, filed, loose = model()
    registry["structure"]["active"] = "sections"
    structure.file(scene, [filed.uid], "sections").do(scene)
    project = Project(scene=scene)
    project.rows()
    return project, filed, loose


def test_tree_click_isolate_restore_and_colour(panel):
    p, filed, loose = panel
    rows = p.action("node", "n:ride")
    assert rows["n:ride"][1][1:3] == [1, 1]
    assert p.scene.selection == {filed}
    set_display(p.win.viewport, color_by="structure")
    assert p.win.viewport.view_tint == {filed.uid: facets.palette("ride")}
    p.action("isolate", "n:ride")
    assert not p.scene.entity_selectable(loose)
    p.action("showall", "")
    assert p.scene._navigator_allowed is None
    assert p.win.viewport.view_tint == {filed.uid: facets.palette("ride")}


def test_drop_names_and_undo(panel):
    p, _, loose = panel
    p.action("fileitem", f"i:{loose.uid}" + wgpu_ui.SEP + "sections")
    assert structure.node_of(loose) == "sections"
    assert loose.layer == "Track Sections" and loose.ifc["class"] == "IfcMember"
    assert len(p.win.viewport.history.undo_stack) == 1
    p.win.viewport.history.undo()
    assert loose.layer is None and loose.ifc is None and structure.node_of(loose) is None


def test_refusal_shows_linked_uid(panel):
    p, _, loose = panel
    loose.ext = {"link": {"path": "linked.igz"}}
    with pytest.raises(ValueError, match=loose.uid):
        commands.run(p.win.viewport.history, "file_placements", placements=[loose.uid], node="sections")
    assert not p.win.viewport.history.undo_stack


def test_unsorted_pages_select_and_file(panel, monkeypatch):
    p, _, loose = panel
    monkeypatch.setattr(wgpu_ui, "ROW_CAP", 200)
    p.scene.groups.extend(Group(name=str(i)) for i in range(201))
    p.scene.version += 1
    rows = p.rows()
    assert rows["n:"][1][1] == 202
    rows = p.action("toggle", "n:")
    assert sum(k.startswith("i:") for k in rows if k[2:] != panel[1].uid) == 200
    p.action("item", f"i:{loose.uid}")
    assert p.scene.selection == {loose}
    more = next(k for k in rows if k.startswith("m:"))
    rows = p.action("loose", more)
    assert more not in rows
    p.action("fileitem", f"i:{loose.uid}" + wgpu_ui.SEP + "ref")
    assert structure.node_of(loose) == "ref"


def test_recipe_validation_and_atomic_resync(panel):
    p, filed, _ = panel
    history = p.win.viewport.history
    old = bim_registry.get(p.scene)["structure"]["nodes"]["sections"]["recipe"]
    with pytest.raises(ValueError, match="unknown class"):
        commands.run(history, "set_filing_rule", node="sections", expected=old, recipe={**old, "class": "BadClass"})
    assert history.undo_stack == [] and filed.ifc["class"] == "IfcMember"
    new = {**old, "class": "IfcColumn", "psets": {"Pset_ColumnCommon": {"Status": {"kind": "enum", "value": "EXISTING"}}}}
    commands.run(history, "set_filing_rule", node="sections", expected=old, recipe=new)
    assert filed.ifc["class"] == "IfcColumn"
    assert filed.ifc["psets"]["Pset_ColumnCommon"]["Status"]["value"] == "EXISTING"
    assert len(history.undo_stack) == 1 and history.undo()
    assert structure.resolve(bim_registry.get(p.scene), "sections")["class"] == filed.ifc["class"] == "IfcMember"


def test_bootstrap_proposal_is_undoable(panel):
    p, _, loose = panel
    registry = bim_registry.get(p.scene)
    p.scene.layers.append(Layer("Survey"))
    loose.layer = "Survey"
    p.action("propose", "")
    assert not structure.node_of(loose)
    p.action("prompt", "yes")
    assert structure.node_of(loose) == "layer:" + p.scene.layer("Survey").uid
    assert len(p.win.viewport.history.undo_stack) == 1 and p.win.viewport.history.undo()
    assert bim_registry.get(p.scene) == registry and not structure.node_of(loose)


def test_filed_placements_refile_through_the_tree_and_reject_direct_tag_edits(panel):
    from ingetrazo_render import inspector_tag_row, tree_edit_tag
    p, filed, _ = panel
    p.scene.select([filed])
    assert structure.path(bim_registry.get(p.scene), "sections") == ["Ride", "Track", "Sections"]
    p.action("fileitem", f"i:{filed.uid}" + wgpu_ui.SEP + "ref")
    assert structure.path(bim_registry.get(p.scene), structure.node_of(filed)) == ["Ride", "Reference"]
    assert not inspector_tag_row(p.scene)
    tree_edit_tag(p.win, "tagassign", "preview-tag:Track Sections", "", set())
    assert filed.layer == "Reference" and len(p.win.viewport.history.undo_stack) == 1


def test_hidden_changes_coalesce_in_one_membership_pass(panel, monkeypatch):
    p, _, _ = panel
    from core import link_structure
    calls = []
    original = link_structure.Tree
    def counted(*args):
        calls.append(True)
        return original(*args)
    monkeypatch.setattr(link_structure, "Tree", counted)
    for _ in range(3):
        p.scene.version += 1
    assert not calls
    p.rows()
    p.scene.bump_view()
    p.rows()
    assert calls == [True]


def test_outliner_group_drag_creates_visible_layer():
    p = Project()
    group = Group(name="Beam")
    p.scene.groups.append(group)
    p.scene.plugin_data["bim_registry"] = {"structure": {"nodes": {
        "beams": {"name": "Beams", "order": 0, "parent": None}}}}
    p.rows()
    p.action("fileitem", "i:" + group.uid + wgpu_ui.SEP + "beams")
    assert p.scene.layer("Beams").visible and group.layer == "Beams"


def test_delete_occupied_leaf_refiles_in_one_step(panel):
    p, filed, _ = panel
    p.action("delete", "n:sections")
    p.action("prompt", "yes")
    assert structure.node_of(filed) == "track" and filed.layer == "Track"
    assert "sections" not in bim_registry.get(p.scene)["structure"]["nodes"]
    p.win.viewport.history.undo()
    assert structure.node_of(filed) == "sections" and filed.layer == "Track Sections"


def test_mounted_rows_and_exact_nested_members(panel):
    p, filed, loose = panel
    leaf = Group(name="Local")
    loose.children = [leaf]
    leaf.ext = {"structure": {"node": "sections"}}
    link = Group(name="Supplier")
    link.ext = {"link": {"path": "ref/Supplier.igz"}}
    p.scene.groups.append(link)
    p.scene.version += 1
    assert any(name == "Supplier.igz" for name, state in p.rows().values())
    p.action("node", "n:sections")
    assert p.scene.selection == {filed, leaf}
    p.action("isolate", "n:sections")
    assert p.scene.entity_selectable(filed) and p.scene.entity_selectable(leaf)


def test_drawing_preserves_structure_rows(panel):
    from core.history import AddEdgeCommand
    from ingetrazo_core import Vec3
    p, _, _ = panel
    tree = p.view._ui_tree["tree"]
    history = p.win.viewport.history
    history.execute(AddEdgeCommand(Vec3(), Vec3(1, 0, 0)))
    for step in (lambda: None, history.undo, history.redo):
        step()
        p.rows()
        assert p.view._ui_tree["tree"] is tree


def test_filing_choices_distinguish_siblings_and_exclude_mounted_descendants():
    from ingetrazo_core import filing_choices
    nodes = {
        "a": {"name": "Floor", "parent": None},
        "b": {"name": "Floor", "parent": None},
        "c": {"name": "Floor (a)", "parent": None},
        "source": {"name": "Linked", "parent": None, "source": "source.igz"},
        "child": {"name": "Walls", "parent": "source"},
    }
    choices = filing_choices(nodes)
    assert list(choices.values()).count("c") == 1
    assert set(choices.values()) == {"a", "b", "c"}
    assert choices["Floor (a)"] == "c"
