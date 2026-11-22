from core.group import Group
from core.mesh import Mesh
from core.scene import Scene
from formats.igz import save_scene, load_into


def test_save_repairs_script_ifc_and_preserves_other_groups(tmp_path, caplog):
    scene = Scene()
    records = ["IfcPipeSegment", 17, {"class": "IfcWall", "invalid": object()},
               {"class": "IfcBeam", "height": 3.5}]
    for i, record in enumerate(records):
        g = Group(Mesh(), f"Part {i}")
        g.ifc = record
        scene.groups.append(g)
    path = tmp_path / "parts.igz"
    save_scene(scene, path)
    restored = Scene()
    load_into(restored, path)
    assert [g.ifc for g in restored.groups] == [
        {"class": "IfcPipeSegment"}, None, {"class": "IfcWall"}, records[3]]
    assert "Part 1" in caplog.text and "Part 2" in caplog.text
    assert scene.groups[0].ifc == records[0]
