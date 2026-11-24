"""Exercise modelling commands through the JSON callables advertised to MCP."""
import json

from core.agent_protocol import ProtocolTools
from core.agent_tools import BUILTIN_TOOLS
from core.scene import Scene
from tests.test_parity_tools import _Vp, V
from tests.test_solid_tools import _pair


def test_registered_model_tools_commit_and_undo_through_the_bridge():
    agent = ProtocolTools()
    vp = agent._viewport = _Vp(Scene())
    catalog = {spec["name"]: spec for spec in BUILTIN_TOOLS}
    a, b = _pair()
    vp.scene.groups.extend([a, b])
    origin, x, y, z = [0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]
    face = vp.scene.mesh.add_face([V(0, 0), V(1, 0), V(1, 1), V(0, 1)])
    edge = vp.scene.mesh.add_edge(V(3, 2), V(4, 2))

    def call(name, **args):
        spec = catalog[name]
        schema = spec["inputSchema"]
        assert spec["mutates"] and set(schema["required"]) <= args.keys() <= schema["properties"].keys()
        handler = getattr(agent, "_tool_" + name)
        depth = len(vp.history.undo_stack)
        result = json.loads(json.dumps(handler(**args)))
        assert result["ok"] and result["step"] and len(vp.history.undo_stack) == depth + 1

    for name, args in [
        ("follow_me", dict(face=str(face.key), path=[origin, z])),
        ("solid_operation", dict(operation="union", placements=[a.uid, b.uid])),
        ("place_group", dict(template=a.uid, basis=[x, y, z], offset=[3, 2, 1])),
        ("set_group_scale", dict(placement=a.uid, factors=[2, 1, 1])),
        ("erase_stroke", dict(edges=[str(edge.key)])),
        ("add_dimension", dict(a=origin, b=x, offset=y)),
        ("add_text_label", dict(anchor=x, offset=y, text="Datum")),
        ("place_section_plane", dict(point=origin, normal=z)),
        ("add_image_plane", dict(path="synthetic.png", origin=origin, u=x, v=y)),
        ("set_north", dict(degrees=35)),
    ]:
        call(name, **args)
        assert vp.history.undo()
    call("add_geo_path", points=[origin, y])
    call("move_geo_path_node", path=0, node=1, point=x)
    assert vp.scene.geo_paths[0].points[1] == V(*x)
    assert vp.history.undo() and vp.scene.geo_paths[0].points[1] == V(*y)
    assert vp.history.undo() and not vp.scene.geo_paths
