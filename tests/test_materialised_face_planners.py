"""Ordered planner answers recorded from the materialised geometry implementation."""
import hashlib
import json
import random

from built_core import Vec3

from built_core import topology
from built_core.geometry import Face


def test_materialised_subdivision_on_random_faces():
    rng = random.Random(928102)
    digest = hashlib.sha256()

    def encode(value):
        if isinstance(value, Vec3):
            return [round(v, 8) for v in value.toTuple()]
        if isinstance(value, (tuple, list)):
            return [encode(v) for v in value]
        return value

    for _ in range(150):
        x, y, z = [rng.uniform(-1e4, 1e4) for _ in range(3)]
        width, height = rng.uniform(.1, 20), rng.uniform(.1, 20)

        def p(u, v):
            return Vec3(x + u * width, y + v * height, z)

        outer = [p(0, 0), p(1, 0), p(1, 1), p(0, 1)]
        if rng.randrange(2):
            outer.reverse()
        inner = [p(.4, .4), p(.6, .4), p(.6, .6), p(.4, .6)]
        face = Face(outer, [inner] if rng.randrange(2) else [])
        for lo, hi in [(0, .25), (.3, .8), (.5, 1), (-.1, .3)]:
            chain = [p(lo, 0), p(lo, .5), p(hi, .5), p(hi, 0)]
            band = [p(lo, 0), p(hi, 0), p(hi, 1), p(lo, 1)]
            results = (topology.loop_inside_face(face, inner),
                       topology.loop_inside_face(face, inner, outside_holes=True),
                       topology.split_face_by_chord(face, band[0], band[-1]),
                       topology.split_face_by_chain(face, chain),
                       topology.find_subdividing_chain(face, chain),
                       topology.subtract_loop_from_face(face, chain),
                       topology.carve_loop_by_chords(face, band))
            digest.update(json.dumps(encode(results)).encode())
    assert digest.hexdigest() == "f41226e7d424433ca55e671445ff141e2fbb807a06c2005a3dd8fae668f62a00"


def test_divide_join_preserve_materialised_handles_and_undo():
    from built_core.edits import divide_edges, join_edges, join_preview
    from built_core.mesh import Mesh, edge_flags

    class Materialised(Mesh):
        pass

    rng = random.Random(10808)
    for _ in range(32):
        mesh = Materialised()
        width, height = rng.uniform(1, 15), rng.uniform(1, 15)
        face = mesh.add_face([Vec3(0, 0, 0), Vec3(width, 0, 0),
                              Vec3(width, height, 0), Vec3(0, height, 0)])
        edge = mesh.edges[0]
        ends = (edge.v0, edge.v1)
        edge.hidden = True
        flags = edge_flags(edge)
        n = rng.randrange(2, 8)
        assert divide_edges(mesh, [edge], n) == n - 1
        pieces = [e for e in mesh.edges if e.hidden]
        assert len(pieces) == n
        assert all(edge_flags(e) == flags for e in pieces)
        assert all(e.faces == [face] for e in pieces)
        snapshot = mesh.capture_state()
        verdicts = join_preview(mesh, pieces)
        assert len(verdicts) == n - 1 and set(verdicts.values()) == {"join"}
        assert all(e in mesh.edges for e in pieces)
        [joined] = join_edges(mesh, pieces)
        assert {joined.v0, joined.v1} == set(ends)
        assert joined.faces == [face] and len(face.loop) == 4
        mesh.restore_state(snapshot)
        assert all(mesh.registers(v) for v in ends)
        assert len(face.loop) == n + 3 and all(e in mesh.edges for e in pieces)


def test_drawn_segments_preserve_the_ordered_command_plan(monkeypatch):
    from built_core.edits import plan_edge_commands
    from built_core.mesh import Mesh
    from built_core.scene import Scene

    class Materialised(Mesh):
        pass

    rng = random.Random(36071)
    digest = hashlib.sha256()

    def coords(v):
        return [round(c, 8) for c in v.toTuple()]

    def signature(command):
        result = [type(command).__name__]
        for field in ("a", "b", "vertices", "holes", "attrs", "auto", "soft", "layer", "curve"):
            value = getattr(command, field, None)
            if hasattr(value, "toTuple"):
                value = coords(value)
            elif field == "vertices" and value is not None:
                value = [coords(p) for p in value]
            elif field == "holes" and value is not None:
                value = [[coords(p) for p in h] for h in value]
            result.append(value)
        return result

    for _ in range(48):
        scene = Scene()
        scene.mesh = Materialised()
        x, y = rng.randint(-80, 80), rng.randint(-80, 80)

        def p(u, v):
            return Vec3(x + u, y + v, 0.)

        corners = [p(0, 0), p(10, 0), p(10, 10), p(0, 10)]
        hole = [p(4, 4), p(6, 4), p(6, 6), p(4, 6)]
        face = scene.mesh.add_face(corners, [hole] if rng.randrange(2) else [])
        face.attrs["material"] = "plaster"
        k = rng.randint(1, 9)
        segments = [(p(0, k), p(10, k)), (p(k, 0), p(k, 10)),
                    (p(10, k), p(k, 10)), (p(0, 0), p(10, 10))]
        for setting in ("0", "1"):
            monkeypatch.setenv("BUILT_SCANS", setting)
            plan = plan_edge_commands(scene, segments, True, p(1, 2))
            digest.update(json.dumps([signature(c) for c in plan]).encode())
        assert scene.mesh.faces == [face]
    assert digest.hexdigest() == "ca1c333fcefaf20580525ddb3ce31b35b28c5ec0d73915f398dfa74e7d0c31dd"
