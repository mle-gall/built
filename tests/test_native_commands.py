"""Native History steps (docs/commands-port.md): placement renames, locks
and whole-placement paint are typed Rust deltas. Execute, undo, redo and
the feed record match the retired Python commands; the protocol verbs are
their front doors."""
import pytest

from core.group import Group
from core.history import CompoundCommand, Command, History, NativeCommand
from core.mesh import Mesh
from core.protocol import commands
from core.scene import Scene


def _scene(n=3):
    scene = Scene()
    gs = [Group(Mesh(), name=f"g{i}", uid=f"u{i}") for i in range(n)]
    scene.groups.extend(gs)
    child = Group(Mesh(), name="inner", uid="inner")
    gs[0].children.append(child)
    return scene, History(scene), gs, child


def test_rename_executes_undoes_and_redoes_by_its_transaction():
    scene, h, gs, _ = _scene()
    version = scene.version
    h.execute(NativeCommand.rename(gs[1], "beam"))
    step = h.undo_stack[-1]
    assert gs[1].name == "beam" and step._doc_txn is not None
    assert scene.version == version + 1
    record = h.feed.last
    assert (record.command, record.phase, record.complete) == ("RenameGroupCommand", "execute", True)
    assert record.placements["modified"] == ["u1"]
    assert h.undo() and gs[1].name == "g1"
    assert h.feed.last.placements["modified"] == ["u1"]
    assert h.redo() and gs[1].name == "beam"


def test_lock_drops_the_targets_from_the_selection_on_execute_and_redo():
    scene, h, gs, _ = _scene()
    scene.selection.add(gs[0])
    scene.selection.add(gs[2])
    h.execute(NativeCommand.lock(gs[:2], True))
    assert all(g.locked for g in gs[:2]) and not gs[2].locked
    assert gs[0] not in scene.selection and gs[2] in scene.selection
    h.undo()
    assert not any(g.locked for g in gs)
    scene.selection.add(gs[1])
    h.redo()
    assert gs[1] not in scene.selection and gs[0].locked
    assert h.feed.last.command == "LockCommand"


def test_unlock_restores_each_previous_flag_exactly():
    scene, h, gs, _ = _scene()
    gs[0].locked = True
    h.execute(NativeCommand.lock(gs, False))
    assert not any(g.locked for g in gs)
    h.undo()
    assert [g.locked for g in gs] == [True, False, False]


def test_paint_sets_and_clears_whole_placement_paint():
    scene, h, gs, child = _scene()
    red = {"color": [1.0, 0.0, 0.0]}
    h.execute(NativeCommand.paint([gs[0], child], red))
    assert gs[0].material == red and child.material == red
    # Like the Python command: the step stays incomplete in the feed.
    assert h.feed.last.command == "SetGroupMaterialCommand" and not h.feed.last.complete
    h.execute(NativeCommand.paint([gs[0]], {}))
    assert gs[0].material is None
    h.undo()
    assert gs[0].material == red
    h.undo()
    assert gs[0].material is None and child.material is None


def test_direct_do_and_undo_use_the_recorded_inverse():
    scene, _, gs, _ = _scene()
    cmd = NativeCommand.rename_each(gs, ["a", "b", "c"])
    cmd.do(scene)
    assert [g.name for g in gs] == ["a", "b", "c"]
    cmd.undo(scene)
    assert [g.name for g in gs] == ["g0", "g1", "g2"]


def test_a_native_step_inside_a_mixed_compound_replays_with_the_host_inverse():
    scene, h, gs, _ = _scene()
    host = []

    class Extension(Command):
        def do(self, scene):
            host.append(gs[0].name)

        def undo(self, scene):
            assert host.pop() == gs[0].name

    h.execute(CompoundCommand([NativeCommand.rename(gs[0], "after"), Extension()]))
    assert host == ["after"] and not h.last_error
    h.undo()
    assert gs[0].name == "g0" and host == []
    h.redo()
    assert gs[0].name == "after" and host == ["after"]


def test_constructors_refuse_what_is_not_a_placement():
    with pytest.raises(TypeError):
        NativeCommand.lock([object()], True)
    scene, _, gs, _ = _scene()
    with pytest.raises(ValueError):
        NativeCommand.rename_each(gs, ["only one"])
    # A repeated placement is written once.
    assert len(NativeCommand.lock([gs[0], gs[0]], True).targets) == 1


def test_a_write_that_changes_nothing_still_makes_a_step():
    scene, h, gs, _ = _scene()
    before = h.revision
    h.execute(NativeCommand.rename(gs[0], "g0"))
    assert h.revision == before + 1 and gs[0].name == "g0"
    assert h.undo() and gs[0].name == "g0"


def test_protocol_verbs_take_uids_or_handles():
    scene, h, gs, child = _scene()
    answer = commands.rename_placement(h, "inner", "renamed")
    assert answer["ok"] and child.name == "renamed"
    assert commands.lock(h, [gs[1]], True)["ok"] and gs[1].locked
    answer = commands.rename_placements(h, ["u0", gs[2]], ["first", "g2"])
    # The unchanged name is left out of the step.
    assert answer["ok"] and gs[0].name == "first"
    assert answer["step"].placements["modified"] == ["u0"]
    with pytest.raises(KeyError):
        commands.lock(h, ["missing"], True)
    with pytest.raises(ValueError):
        commands.rename_placements(h, ["u0"], ["a", "b"])


def test_paint_placements_registers_a_new_material_in_the_same_step():
    from core.materials import Material
    scene, h, gs, _ = _scene()
    mat = Material("Oak", color=(0.6, 0.4, 0.2))
    paint = {"color": [0.6, 0.4, 0.2], "mat": "Oak"}
    answer = commands.run(h, "paint_placements", placements=["u0", "u1"], paint=paint, material=mat)
    assert answer["ok"] and scene.materials.get("Oak") is not None
    assert gs[0].material == paint and gs[1].material == paint
    assert len(h.undo_stack) == 1
    h.undo()
    assert scene.materials.get("Oak") is None and gs[0].material is None


def test_rename_component_is_one_native_step():
    from built_core.kept_components import copies_of
    scene = Scene()
    h = History(scene)
    mesh = Mesh()
    a, b = Group(mesh, name="Door", uid="a"), Group(mesh, name="Door", uid="b")
    from built_core import Matrix4
    for g in (a, b):
        g.xform, g.component = Matrix4(), True
        scene.groups.append(g)
    assert len(copies_of(scene, mesh)) == 2
    answer = commands.rename_component(h, "a", "Gate")
    assert answer["ok"] and a.name == b.name == "Gate"
    assert answer["step"].command == "RenameGroupCommand"
    assert sorted(answer["step"].placements["modified"]) == ["a", "b"]
    h.undo()
    assert a.name == b.name == "Door"


# ---- record steps (annotations, sections, guides, images, scenes, sheets)

def _guide_scene():
    from built_core import Vec3
    from core.guide import Guide
    scene = Scene()
    guide = Guide(Vec3(0, 0, 0), Vec3(1, 0, 0))
    return scene, History(scene), guide, Vec3


def test_record_steps_are_native_history_commands():
    import built_core
    from core import history as H
    for name in ("AddGuideCommand", "DeleteDimensionsCommand", "CreatePlanCommand",
                 "UpdateSceneCommand", "MoveGeoPathNodeCommand"):
        cls = getattr(H, name)
        assert cls is getattr(built_core, name)
        assert issubclass(cls, built_core.RecordStep) and issubclass(cls, Command)


def test_a_record_step_undoes_by_its_transaction_in_history():
    scene, h, guide, _ = _guide_scene()
    from core.history import AddGuideCommand
    step = AddGuideCommand(guide)
    assert step.guide is guide and step.keeps_rows and step.keeps_paint
    h.execute(step)
    assert list(scene.guides) == [guide] and step._doc_txn is not None
    assert h.feed.last.command == "AddGuideCommand"
    scene.selection.add(guide)
    h.undo()
    assert list(scene.guides) == [] and guide not in scene.selection
    h.redo()
    assert list(scene.guides) == [guide]


def test_a_direct_record_step_replays_the_transaction_it_opened():
    scene, _, guide, Vec3 = _guide_scene()
    from core.history import AddGuideCommand, MoveGuidesCommand
    AddGuideCommand(guide).do(scene)
    move = MoveGuidesCommand([guide], Vec3(0, 2, 0))
    move.do(scene)
    assert guide.point.y() == 2
    move.undo(scene)
    assert guide.point.y() == 0 and list(scene.guides) == [guide]


def test_a_record_step_inside_a_compound_with_its_own_inverse_undoes():
    scene, h, guide, Vec3 = _guide_scene()
    from core.history import AddGuideCommand, CompoundCommand

    class OwnInverse(CompoundCommand):
        doc_undo = False   # as the Move tool's copies

    h.execute(OwnInverse([AddGuideCommand(guide)]))
    assert list(scene.guides) == [guide]
    h.undo()
    assert list(scene.guides) == []
    h.redo()
    assert list(scene.guides) == [guide]


def test_a_rejected_record_step_leaves_nothing_behind():
    from core.history import SceneTreeCommand
    scene = Scene()
    h = History(scene)
    before = h.revision
    h.execute(SceneTreeCommand(lambda scene: False))
    assert h.revision == before and h.last_rejected and not h.undo_stack


def test_the_tool_preview_helpers_stay_on_the_classes():
    from core.history import MoveGuidesCommand, RotateGuidesCommand
    scene, _, guide, Vec3 = _guide_scene()
    MoveGuidesCommand.shift(guide, Vec3(1, 0, 0))
    assert guide.point.x() == 1
    state = RotateGuidesCommand.state(guide)
    RotateGuidesCommand.place(state, None)
    assert guide.point.x() == 1


# ---- group and component steps

def test_insert_and_delete_publish_where_the_placement_went():
    from core.history import DeleteGroupCommand, InsertGroupCommand
    import built_core
    scene = Scene()
    h = History(scene)
    g = Group(Mesh(), name="part", uid="p")
    step = InsertGroupCommand(g)
    assert isinstance(step, built_core.RecordStep) and step.keeps_groups
    assert step._owner is None and step.group is g
    h.execute(step)
    assert step._owner is scene.groups and g in scene.selection
    gone = DeleteGroupCommand(g)
    h.execute(gone)
    assert g not in scene.groups and gone.index == 0
    h.undo()
    assert list(scene.groups) == [g]
    h.undo()
    assert list(scene.groups) == [] and g not in scene.selection
    h.redo()
    h.redo()
    assert list(scene.groups) == []


def test_merge_puts_back_the_same_merged_group_on_redo():
    from core.history import MergeGroupsCommand
    from built_core import Vec3
    scene = Scene()
    h = History(scene)
    parts = []
    for x in (0, 2):
        m = Mesh()
        m.add_face([Vec3(x, 0, 0), Vec3(x + 1, 0, 0), Vec3(x + 1, 1, 0)])
        parts.append(Group(m, name=f"g{x}"))
    scene.groups.extend(parts)
    h.execute(MergeGroupsCommand(parts))
    merged = scene.groups[0]
    assert len(scene.groups) == 1 and merged not in parts
    h.undo()
    assert list(scene.groups) == parts
    h.redo()
    assert list(scene.groups) == [merged]


# ---- mesh steps

def _square(mesh, x=0.0):
    from built_core import Vec3
    return mesh.add_face([Vec3(x, 0, 0), Vec3(x + 1, 0, 0), Vec3(x + 1, 1, 0), Vec3(x, 1, 0)])


def test_make_group_is_a_native_mesh_step_with_one_group_across_redo():
    from core.history import MakeGroupCommand, MeshCommand
    scene = Scene()
    h = History(scene)
    face = _square(scene.mesh)
    step = MakeGroupCommand([face], list(scene.mesh.edges))
    assert isinstance(step, MeshCommand) and step.group is None
    assert step.geometry_meshes(scene) == (scene.mesh,)
    h.execute(step)
    group = step.group
    assert list(scene.groups) == [group] and not scene.mesh.faces
    h.undo()
    assert list(scene.groups) == [] and len(scene.mesh.faces) == 1
    h.redo()
    assert list(scene.groups) == [group] and not scene.mesh.faces


def test_nested_group_and_component_of_hold_the_groups():
    from core.history import MakeComponentOfCommand, MakeNestedGroupCommand
    scene = Scene()
    h = History(scene)
    a, b = Group(Mesh(), name="a"), Group(Mesh(), name="b")
    scene.groups.extend([a, b])
    h.execute(MakeNestedGroupCommand([], [], [a, b], name="both"))
    box = h.undo_stack[-1].container
    assert list(scene.groups) == [box] and list(box.children) == [a, b]
    h.undo()
    assert list(scene.groups) == [a, b]
    h.execute(MakeComponentOfCommand([], [], [a, b]))
    comp = h.undo_stack[-1].container
    assert comp.component and list(scene.groups) == [comp]


def test_plugin_compound_state_survives_copy_and_pickle():
    from copy import deepcopy
    from pickle import dumps, loads
    from core.history import SetPluginDataCommand

    command = CompoundCommand([SetPluginDataCommand("extension", {"tags": ["A"]})])
    for restored in (deepcopy(command), loads(dumps(command))):
        scene = Scene()
        restored.do(scene)
        assert scene.plugin_data["extension"] == {"tags": ["A"]}
        restored.undo(scene)
        assert "extension" not in scene.plugin_data
        assert restored.commands[0].value is not command.commands[0].value
