"""One document scenario, run by tests/test_document_parity.py in two
processes: ``INGETRAZO_DOC=0`` (the Python document) and the default (the
Rust document, core/doc_model.py). Prints one JSON object: the document
after every step, the feed's records, and what the saved files hold.

    python tests/doc_parity_scenario.py <workdir>
"""
from __future__ import annotations

import hashlib
import json
import os
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtGui import QMatrix4x4, QVector3D  # noqa: E402

from core import doc_model  # noqa: E402
from core.group import Group  # noqa: E402
from core.history import (AssignLayerCommand, DeleteGroupCommand,  # noqa: E402
                          HideCommand, History, InsertGroupCommand,
                          LockCommand, MakeNestedGroupCommand, MoveGroupCommand,
                          RenameGroupCommand, SetGroupMaterialCommand,
                          SetLayerVisibilityCommand, SetMaterialFinishCommand)
from core.layers import TAG_FIELDS, Layer  # noqa: E402
from core.materials import Material  # noqa: E402
from core.mesh import Mesh  # noqa: E402
from core.scene import Scene  # noqa: E402
from formats import built, igz  # noqa: E402
import core.group as _group  # noqa: E402
import core.layers as _layers  # noqa: E402
import itertools  # noqa: E402

# Uids the two runs agree on.
_uids = itertools.count(1)
_group.new_uid = _layers.new_uid = lambda: f"uid-{next(_uids):05d}"


def box(mesh, s=1.0, x=0.0):
    import numpy as np
    p = [(0, 0, 0), (s, 0, 0), (s, s, 0), (0, s, 0),
         (0, 0, s), (s, 0, s), (s, s, s), (0, s, s)]
    quads = [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4),
             (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
    pos = np.array([p[i] for q in quads for i in q], dtype=np.float64)
    pos[:, 0] += x
    mesh.add_faces_bulk(pos, np.full(6, 4), np.ones(6, dtype=np.int64))


def matrix(m):
    return None if m is None else [float(v) for v in m.data()]


def group_dump(g):
    return {
        "uid": g.uid, "name": g.name, "layer": g.layer,
        "hidden": bool(g.hidden), "locked": bool(g.locked),
        "component": bool(g.component), "material": g.material,
        "xform": matrix(g.xform), "axes": matrix(g.axes),
        "built": (built.placement_values(g.xform)
                  if g.xform is not None else None),
        "faces": len(g.mesh.faces),
        "children": [group_dump(c) for c in g.children],
    }


def dump(scene):
    return json.loads(json.dumps({
        "groups": [group_dump(g) for g in scene.groups],
        "layers": [dict({f: getattr(ly, f) for f in TAG_FIELDS},
                        **{"dict": ly.to_dict()}) for ly in scene.layers],
        "materials": {k: m.to_dict() for k, m in sorted(scene.materials.items())},
        "loose_faces": len(scene.mesh.faces),
        "active_layer": scene.active_layer,
    }))


def build():
    scene = Scene()
    for i in range(3):
        scene.layers.append(Layer(f"Tag {i}", uid=f"tag-uid-{i}",
                                  color=(i / 3, 0.5, 0.25)))
    for i in range(2):
        scene.materials[f"Mat {i}"] = Material(f"Mat {i}", color=(0.1 * i, 0.2, 0.3))
    defs = []
    for d in range(2):
        m = Mesh()
        box(m, 0.5 + d * 0.25)
        defs.append(m)
    kids = []
    for k in range(4):
        g = Group(defs[k % 2], name=f"Part {k}", uid=f"part-{k}")
        t = QMatrix4x4()
        t.translate(QVector3D(k * 1.5, 0.25, 0))
        t.rotate(15.0 * k, QVector3D(0, 0, 1))
        g.xform = t
        g.layer = f"Tag {k % 3}"
        kids.append(g)
    assembly = Group(Mesh(), name="Assembly", uid="assembly")
    assembly.adopt(kids)
    assembly.component = False
    scene.groups.append(assembly)
    # A .built placement: binary64 values beside their float32 projection.
    vals = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 650000.013, 6860000.027, 1.5, 1]
    inst = Group(defs[0], name="Site", uid="site")
    inst.xform = built.placement(vals)
    scene.groups.append(inst)
    classic = Mesh()
    box(classic, 1.0, x=10.0)
    scene.groups.append(Group(classic, name="Classic", uid="classic"))
    return scene


def step_record(step):
    return {"phase": step.phase, "command": step.command,
            "placements": {k: sorted(v) for k, v in step.placements.items()},
            "tags": step.tags, "materials": step.materials,
            "complete": step.complete}


def members(path):
    """Each member's bytes (a ZIP's own timestamps left out); a small
    document is one plain JSON file."""
    if not zipfile.is_zipfile(path):
        return {"document": hashlib.sha256(Path(path).read_bytes()).hexdigest()}
    with zipfile.ZipFile(path) as z:
        return {n: hashlib.sha256(z.read(n)).hexdigest()
                for n in sorted(z.namelist())}


def saved(scene, work: Path, tag: str) -> dict:
    files = {}
    for name in ("doc.igz", "doc.built"):
        path = work / f"{tag}-{name}"
        igz.save_scene(scene, path, backup=False)
        back = Scene()
        igz.load_into(back, path)
        files[name] = {"members": members(path), "reopened": dump(back)}
        # Saved again from the reopened document: the same bytes.
        again = work / f"{tag}-again-{name}"
        igz.save_scene(back, again, backup=False)
        files[name]["again"] = members(again)
    return files


def main(work: Path):
    initial = saved(build(), work, "initial")
    scene = build()
    h = History(scene)
    steps = []
    h.feed.subscribe(lambda s: steps.append(step_record(s)))
    states = [dump(scene)]
    a = scene.groups[0]
    part = a.children[1]
    extra = Group(Mesh(), name="Inserted", uid="inserted")
    cmds = [
        RenameGroupCommand(part, "Renamed part"),
        LockCommand([scene.groups[1]], True),
        HideCommand([scene.groups[2], part], True),
        AssignLayerCommand([scene.groups[1]], "Brand new tag"),
        SetGroupMaterialCommand(part, {"color": (1.0, 0.0, 0.0), "mat": "Mat 1"}),
        MoveGroupCommand(part, QVector3D(0.1, 0.2, 0.3)),
        MoveGroupCommand(scene.groups[2], QVector3D(0.0, 1.0, 0.0)),
        InsertGroupCommand(extra),
        DeleteGroupCommand(scene.groups[1]),
        SetLayerVisibilityCommand(["Tag 1"], False),
        SetMaterialFinishCommand("Mat 0", "matte"),
    ]
    for c in cmds:
        h.execute(c)
        assert not h.last_error, h.last_error
        states.append(dump(scene))
    while h.undo():
        states.append(dump(scene))
    for _ in range(len(cmds)):
        h.redo()
        states.append(dump(scene))
    for _ in range(5):
        h.undo()
        states.append(dump(scene))
    # A group of two root placements (a structural command with its own
    # undo: journalled, not undone by its transaction).
    h.execute(MakeNestedGroupCommand([], [], list(scene.groups[:2]), name="Outer"))
    assert not h.last_error, h.last_error
    states.append(dump(scene))
    h.undo()
    states.append(dump(scene))
    h.redo()
    states.append(dump(scene))
    print(json.dumps({"engine": doc_model.engine(), "states": states,
                      "feed": steps, "initial": initial,
                      "final": saved(scene, work, "final")}))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
