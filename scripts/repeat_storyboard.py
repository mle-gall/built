"""Build the portal-frame Repeat storyboard in the requested output directory."""
import sys
from pathlib import Path

from built_core import Matrix4, Vec3
from built_core.group import Group
from built_core.history import History
from built_core.protocol import commands, queries
from built_core.scene import Scene
from formats import igz


def build(folder):
    scene = Scene()
    history = History(scene)

    def apply(verb, **args):
        result = commands.run(history, verb, **args)
        if not result["ok"]:
            raise ValueError(result)

    def member(name, start, end, profile="basic/rectangle", **params):
        spec = {"profile": {"library": profile, "params": params},
                "path": {"points": [start, end]}, "attachment": "centre"}
        apply("create_sweeps", sweeps=[spec])
        g = scene.groups[-1]
        g.name = name
        if profile == "basic/rectangle":
            g.material = {"mat": "Glulam", "color": [.74, .53, .32]}
        else:
            g.material = {"mat": "Wall panel", "color": [.80, .81, .79]}
        return g

    def repeat(source, uid, delta, count):
        apply("live_set", row={"kind": "repeat", "uid": f"{uid:016x}", "source": source.uid,
                               "run": {"points": [[0, 0, 0], delta]}, "rule": {"divide": count}})
        return scene.groups[-1]

    def host_face(g, side):
        face = max(g.mesh.faces, key=lambda f: side * f.normal().y())
        pid = g.mesh._store.id_of("face", face.key)
        from built_core import element_ids
        return {"path": [g.uid], "id": element_ids.id_text("face", pid)}

    frame = Group(name="Portal frame")
    frame.xform = Matrix4()
    frame.ifc = {"class": "IfcElementAssembly", "name": "Portal frame"}
    scene.groups.append(frame)
    for name, a, b in [
        ("West column", (0, 0, 0), (0, 0, 3.2)), ("East column", (7, 0, 0), (7, 0, 3.2)),
        ("West rafter", (0, 0, 3.2), (3.5, 0, 4.5)), ("East rafter", (3.5, 0, 4.5), (7, 0, 3.2))
    ]:
        piece = member(name, a, b, width=145, height=240)
        scene.groups.remove(piece)
        frame.children.append(piece)
    folder.mkdir(exist_ok=True, parents=True)
    igz.save_scene(scene, folder / "source.built")
    repeat(frame, 100, [0, 9, 0], 3).name = "Portal frames"
    igz.save_scene(scene, folder / "frames.built")

    near = member("South bearing", (0, -.2, 3.38), (3.5, -.2, 4.68), width=100, height=160)
    far = member("North bearing", (0, 9.1, 3.38), (3.5, 9.6, 4.68), width=100, height=160)
    part = member("Roof purlin", (.1, 0, 3.45), (.1, 9, 3.45), width=90, height=140)
    row = next(r for r in queries.live_rows(scene) if r["owner"] == part.uid)
    ends = [host_face(near, 1), host_face(far, -1)]
    row["path"]["ends"] = [{"to": ref, "offset": 20, "extend": True} for ref in ends]
    apply("live_set", row=row)
    for i, ref in enumerate(ends):
        apply("live_set", row={"kind": "cut", "uid": f"{201+i:016x}", "owner": part.uid,
                              "operation": "trim", "end": ["start", "end"][i],
                              "cutter": {"plane": {}}, "anchor": {"ref": ref, "offset": 20}})
    repeat(part, 210, [3.25, 0, 1.21], 5).name = "Adaptive roof purlins"
    east = member("East roof purlin", (3.6, -.24, 4.65), (3.6, 9.38, 4.65), width=90, height=140)
    repeat(east, 211, [3.2, 0, -1.19], 5)
    member("West wall", (0, 0, 1.15), (0, 9, 1.15), "basic/wall", height=2300, thickness=130)
    wall = member("Entry wall", (.05, 0, 1.15), (6.95, 0, 1.15), "basic/wall", height=2300, thickness=130)
    door = Group(name="Entrance door")
    door.xform = Matrix4()
    door.ifc = {"class": "IfcDoor", "name": "Entrance door"}
    door.ext = {"library_host": "wall"}
    front = [Vec3(x, 0, z) for x, z in [(0, 0), (1.1, 0), (1.1, 2.1), (0, 2.1)]]
    back = [v + Vec3(0, .045, 0) for v in front]
    door.mesh.add_face(front)
    door.mesh.add_face(back[::-1])
    for i in range(4):
        j = (i + 1) % 4
        door.mesh.add_face([front[j], front[i], back[i], back[j]])
    for face in door.mesh.faces:
        face.attrs["color"] = (.32, .46, .55)
    apply("place_group", template=door.uid, _pending=door,
          basis=[[1, 0, 0], [0, 1, 0], [0, 0, 1]], offset=[4.1, -.065, 0],
          wall_host={"uid": wall.uid, "normal": [0, -1, 0], "initial_offset": 0,
                     "offset": 0, "sill": 0, "flipped": False,
                     "outline": [[4.1, -.065, 0], [5.2, -.065, 0], [5.2, -.065, 2.1], [4.1, -.065, 2.1]]})
    for row in queries.live_rows(scene):
        if row["state"] != "ok":
            raise ValueError((row["uid"], row["state"]))
    igz.save_scene(scene, folder / "portal.built")


if __name__ == "__main__":
    build(Path(sys.argv[1]))
